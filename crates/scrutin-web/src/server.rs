//! Axum `Router` assembly, loopback middleware, and CORS headers.

use std::net::SocketAddr;

use axum::Router;
use axum::extract::ConnectInfo;
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};

use crate::routes;
use crate::state::AppState;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .merge(routes::api_router())
        .merge(routes::static_router())
        .layer(middleware::from_fn(cors_and_loopback))
        .with_state(state)
}

/// Combined middleware: reject non-loopback peers, add permissive CORS
/// headers (needed for VS Code WebViews whose origin is
/// `vscode-webview://<uuid>`), and handle preflight OPTIONS requests.
async fn cors_and_loopback(req: Request<axum::body::Body>, next: Next) -> Result<Response, StatusCode> {
    let loopback = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip().is_loopback())
        .unwrap_or(false);
    if !loopback {
        return Err(StatusCode::FORBIDDEN);
    }

    // Preflight: return 204 with CORS headers without forwarding.
    if req.method() == Method::OPTIONS {
        return Ok(cors_headers(
            StatusCode::NO_CONTENT.into_response(),
        ));
    }

    let response = next.run(req).await;
    Ok(cors_headers(response))
}

fn cors_headers(mut resp: Response) -> Response {
    let h = resp.headers_mut();
    h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, OPTIONS"));
    h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("*"));
    resp
}
