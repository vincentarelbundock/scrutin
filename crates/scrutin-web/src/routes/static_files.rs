//! Static-asset handler. Serves embedded frontend files with SPA fallback
//! to `index.html` for any unknown path that lacks an extension.

use axum::Router;
use axum::body::Body;
use axum::extract::Path;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::assets;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/{*path}", get(asset))
}

async fn index() -> Response {
    serve("index.html")
}

async fn asset(Path(path): Path<String>) -> Response {
    if assets::get(&path).is_some() {
        serve(&path)
    } else if !path.contains('.') {
        serve("index.html")
    } else {
        (StatusCode::NOT_FOUND, "not found").into_response()
    }
}

fn serve(path: &str) -> Response {
    let Some(file) = assets::get(path) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static(file.mime),
        )
        .header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))
        .body(Body::from(file.data))
        .unwrap()
}
