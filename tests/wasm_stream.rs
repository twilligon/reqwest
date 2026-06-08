//! WASM streaming-body tests (require the `stream` feature and a browser).
//!
//! These run against a local HTTP/2 + TLS echo server — Chromium only streams a
//! request body over h2 — so the suite is hermetic (no external network). Run
//! them through the wrapper that starts that server:
//!
//!   tests/wasm-stream-test.sh --headless --chrome  --test wasm_stream --features stream
//!   tests/wasm-stream-test.sh --headless --firefox --test wasm_stream --features stream
//!
//! Streaming *request* bodies are Chromium-only as of 2026 (HTTP/2 + HTTPS +
//! `duplex: "half"`); on Firefox/Safari reqwest transparently buffers the body,
//! so both paths post the same bytes and the echo assertion holds either way.
#![cfg(all(target_arch = "wasm32", feature = "stream"))]

use futures_util::{stream, StreamExt};
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

// Local HTTP/2 + TLS echo server. tests/wasm_stream_runner.rs injects its
// address here via REQWEST_TEST_ECHO_URL; the default matches a manually started
// server on the conventional port.
const ECHO: &str = match option_env!("REQWEST_TEST_ECHO_URL") {
    Some(url) => url,
    None => "https://127.0.0.1:9443/",
};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

/// Streaming *response*: consume `Response::bytes_stream()` chunk by chunk.
#[wasm_bindgen_test]
async fn stream_response_bytes() {
    let res = reqwest::get(ECHO).await.expect("GET echo server");
    assert!(res.status().is_success(), "status = {}", res.status());

    let mut body = res.bytes_stream();
    let mut collected: Vec<u8> = Vec::new();
    let mut chunks = 0usize;
    while let Some(item) = body.next().await {
        collected.extend_from_slice(&item.expect("bytes_stream yielded an error"));
        chunks += 1;
    }
    log(&format!(
        "stream_response_bytes: {} bytes in {} chunk(s)",
        collected.len(),
        chunks
    ));
    assert!(!collected.is_empty(), "streamed response body was empty");
    let text = String::from_utf8_lossy(&collected);
    assert!(text.contains("data"), "unexpected response body: {text}");
}

/// Streaming *request*: upload a `Body::wrap_stream(..)`; the echo server returns
/// the received bytes in a JSON `data` field. On Chromium this streams with
/// `duplex: "half"` over h2; elsewhere reqwest buffers it first — either way the
/// bytes arrive intact.
#[wasm_bindgen_test]
async fn stream_request_upload() {
    let chunks: Vec<Result<&'static str, std::io::Error>> =
        vec![Ok("hello"), Ok(" "), Ok("streamed"), Ok(" "), Ok("world")];
    let body = reqwest::Body::wrap_stream(stream::iter(chunks));

    let res = reqwest::Client::new()
        .post(ECHO)
        .header("content-type", "text/plain")
        .body(body)
        .send()
        .await
        .expect("streamed POST should succeed");
    assert!(res.status().is_success(), "status = {}", res.status());

    let text = res.text().await.expect("read echo response");
    log(&format!("stream_request_upload: {} byte echo", text.len()));
    assert!(
        text.contains("hello streamed world"),
        "server did not echo the streamed body; response = {text}"
    );
}
