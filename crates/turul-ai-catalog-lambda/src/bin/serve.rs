//! Local development server: the Lambda's handler on a plain HTTP port.
//!
//! ```text
//! turul-ai-catalog-serve [--file PATH] [--addr HOST:PORT]
//! ```
//!
//! Serves PATH (default: the bundled catalog) at `/.well-known/ai-catalog.json` on
//! `127.0.0.1:8080` by default. The file passes the same validation gate as Lambda cold start,
//! and every request is answered by the same `handle`.

use std::net::SocketAddr;
use std::process::ExitCode;

use axum::body::{Body as AxumBody, to_bytes};
use axum::extract::Request as AxumRequest;
use axum::response::Response as AxumResponse;
use lambda_http::Body;
use turul_ai_catalog::identity::WELL_KNOWN_PATH;
use turul_ai_catalog_lambda::{BUNDLE, ServedCatalog, handle};

const USAGE: &str = "usage: turul-ai-catalog-serve [--file PATH] [--addr HOST:PORT]";

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let (file, addr) = match args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let bytes: &'static [u8] = match &file {
        None => BUNDLE,
        Some(path) => match std::fs::read(path) {
            Ok(b) => Box::leak(b.into_boxed_slice()),
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                return ExitCode::from(2);
            }
        },
    };
    let served: &'static ServedCatalog = match ServedCatalog::load(bytes) {
        Ok(s) => Box::leak(Box::new(s)),
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };

    let app = axum::Router::new().fallback(move |req: AxumRequest| route(served, req));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot bind {addr}: {e}");
            return ExitCode::from(2);
        }
    };
    let source = file.as_deref().unwrap_or("bundled catalog");
    eprintln!("serving {source} at http://{addr}{WELL_KNOWN_PATH}");
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    match axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("server error: {e}");
            ExitCode::from(1)
        }
    }
}

/// Adapts an HTTP request to the Lambda handler and its response back; routing stays in `handle`.
async fn route(served: &'static ServedCatalog, req: AxumRequest) -> AxumResponse {
    let (parts, body) = req.into_parts();
    // The handler routes on method and path only; the body is drained, never read.
    let _ = to_bytes(body, 64 * 1024).await;
    let request = lambda_http::Request::from_parts(parts, Body::Empty);
    let (parts, body) = handle(served, &request).into_parts();
    let body = match body {
        Body::Empty => AxumBody::empty(),
        Body::Text(t) => AxumBody::from(t),
        Body::Binary(b) => AxumBody::from(b),
        _ => AxumBody::empty(),
    };
    AxumResponse::from_parts(parts, body)
}

fn args() -> Result<(Option<String>, SocketAddr), String> {
    let mut file = None;
    let mut addr: SocketAddr = "127.0.0.1:8080".parse().expect("valid default address");
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--file" => file = Some(value),
            "--addr" => addr = value.parse().map_err(|e| format!("--addr {value}: {e}"))?,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok((file, addr))
}
