//! HTTP route assembly. Two subrouters (API + static) composed at the
//! top level by `server::build_router`.

mod control;
mod events;
mod snapshot;
mod static_files;

use axum::Router;

use crate::state::AppState;

pub fn api_router() -> Router<AppState> {
    Router::new()
        .merge(snapshot::router())
        .merge(events::router())
        .merge(control::router())
}

pub fn static_router() -> Router<AppState> {
    static_files::router()
}
