//! SSE stream at `GET /api/events`.
//!
//! One forwarder task per run writes to `AppState::events_tx`
//! (`tokio::sync::broadcast`). Each browser tab subscribes a receiver
//! and the SSE handler turns the stream into `text/event-stream` frames.
//! Late-joining clients miss nothing they care about: they fetch
//! `/api/snapshot` first, then subscribe here. Slow consumers drop
//! oldest events and recover via `Last-Event-ID` replay from
//! `AppState::replay_buffer`.

use std::convert::Infallible;

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use futures::stream::{self, Stream, StreamExt};
use tokio_stream::wrappers::BroadcastStream;

use crate::state::{AppState, SeqEvent};

pub fn router() -> Router<AppState> {
    Router::new().route("/api/events", get(events_stream))
}

async fn events_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Parse Last-Event-ID (if any) so we can replay buffered events the
    // client missed during its disconnect.
    let last_id: Option<u64> = headers
        .get("Last-Event-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());

    // 1. Bootstrap: replay buffered events with id > last_id (or all if None).
    let replay: Vec<SeqEvent> = {
        let buf = state.replay_buffer.read().await;
        buf.iter()
            .filter(|e| last_id.map(|lid| e.id > lid).unwrap_or(true))
            .cloned()
            .collect()
    };

    // 2. Live stream from the broadcast channel. Heartbeats are published
    // from the state-level task in `AppState::new` (so they carry busy
    // worker count); the SSE layer doesn't generate its own.
    let rx = state.events_tx.subscribe();
    let live = BroadcastStream::new(rx).filter_map(|r| async { r.ok() });

    let combined = stream::iter(replay).chain(live);
    let mapped = combined.map(|seq_ev| {
        let kind = seq_ev.event.kind_str();
        let data =
            serde_json::to_string(&seq_ev.event).unwrap_or_else(|_| "{}".to_string());
        let mut ev = Event::default().event(kind).data(data);
        if seq_ev.id != u64::MAX {
            ev = ev.id(seq_ev.id.to_string());
        }
        Ok(ev)
    });

    Sse::new(mapped).keep_alive(KeepAlive::new())
}
