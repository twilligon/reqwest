//! Native driver for the wasm streaming test (`tests/wasm_stream.rs`).
//!
//! This is `#[ignore]`d, so it never runs in the default suite — it launches a
//! headless browser via `wasm-pack`. It boots an in-process HTTP/2 + TLS echo
//! server (Chromium only streams a request body over h2) using the shared
//! `support::server::https` helper, then runs `wasm-pack test` against it,
//! passing the server's address through `REQWEST_TEST_ECHO_URL`. Everything is
//! hermetic and orchestrated in Rust — no shell script, no external network.
//!
//! Run it explicitly (needs `wasm-pack`, a chromedriver, and the wasm target):
//!
//!   cargo test --test wasm_stream_runner -- --ignored --nocapture
#![cfg(not(target_arch = "wasm32"))]

mod support;

use std::process::Command;

use http_body_util::BodyExt;

/// Echo the request body back as `{"data": "<body>"}`, with wide-open CORS so the
/// wasm test page (a different origin) can read the response.
async fn echo(req: http::Request<hyper::body::Incoming>) -> http::Response<reqwest::Body> {
    fn with_cors(b: http::response::Builder) -> http::response::Builder {
        b.header("access-control-allow-origin", "*")
            .header(
                "access-control-allow-methods",
                "GET, POST, PUT, DELETE, OPTIONS",
            )
            .header("access-control-allow-headers", "content-type")
    }

    if req.method() == http::Method::OPTIONS {
        return with_cors(http::Response::builder())
            .status(204)
            .body(reqwest::Body::from(""))
            .unwrap();
    }

    let body = req
        .into_body()
        .collect()
        .await
        .map(|c| c.to_bytes())
        .unwrap_or_default();
    let json = format!(
        "{{\"data\":{}}}",
        json_string(&String::from_utf8_lossy(&body))
    );
    with_cors(http::Response::builder())
        .header("content-type", "application/json")
        .body(reqwest::Body::from(json))
        .unwrap()
}

/// Serialize `s` as a JSON string literal (quotes included).
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[test]
#[ignore = "launches a headless browser via wasm-pack; run explicitly with --ignored"]
fn wasm_streaming_in_browser() {
    let server = support::server::https(|req| echo(req));
    let echo_url = format!("https://{}/", server.addr());
    let webdriver_path = write_webdriver_config();

    // Chrome streams the upload; Firefox lacks request streaming, so it exercises
    // reqwest's buffering fallback — and the echo assertion proves it sent the real
    // bytes rather than a stringified "[object ReadableStream]".
    for (browser, classic_worker) in [("--chrome", false), ("--firefox", true)] {
        let mut cmd = Command::new("wasm-pack");
        cmd.args([
            "test",
            "--headless",
            browser,
            "--features",
            "stream",
            "--test",
            "wasm_stream",
        ])
        .env("REQWEST_TEST_ECHO_URL", &echo_url)
        .env("WASM_BINDGEN_TEST_WEBDRIVER_JSON", &webdriver_path);
        // Firefox's wasm-bindgen-test harness can't use module service workers.
        if classic_worker {
            cmd.env("WASM_BINDGEN_USE_NO_MODULE", "1");
        }
        let status = cmd
            .status()
            .expect("run `wasm-pack test` (is wasm-pack installed?)");
        assert!(status.success(), "wasm-pack test failed on {browser}");
    }
}

/// Write a throwaway WebDriver config that trusts the self-signed cert and runs
/// headless. Chromium refuses to run as root without `--no-sandbox`, so add it
/// only then — CI runs as a normal user and won't get it.
fn write_webdriver_config() -> std::path::PathBuf {
    let no_sandbox = if unsafe { libc::getuid() } == 0 {
        r#""--no-sandbox", "#
    } else {
        ""
    };
    let json = format!(
        r#"{{"acceptInsecureCerts":true,"goog:chromeOptions":{{"args":[{no_sandbox}"--headless","--ignore-certificate-errors","--disable-gpu","--disable-dev-shm-usage"]}},"moz:firefoxOptions":{{"args":["-headless"]}}}}"#
    );
    let path = std::env::temp_dir().join("reqwest-wasm-stream-webdriver.json");
    std::fs::write(&path, json).expect("write webdriver.json");
    path
}
