//! HTTP API for the Apps panel and app runtime.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use serde::{Deserialize, Serialize};

use super::service::{AppError, AppService};
use crate::serve::state::ServeState;

#[derive(Serialize)]
struct ApiOk<T: Serialize> {
    ok: bool,
    data: T,
}

#[derive(Serialize)]
struct ApiErr {
    ok: bool,
    error: String,
}

#[derive(Serialize)]
struct AppIdData {
    id: String,
}

#[derive(Serialize)]
struct ReadmeData {
    id: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct TrustBody {
    definition_hash: String,
}

pub fn router() -> Router<ServeState> {
    Router::new()
        .route("/", get(list_apps))
        .route("/{id}/readme", get(get_readme))
        .route("/{id}/install", post(post_install))
        .route("/{id}/start", post(post_start))
        .route("/{id}/stop", post(post_stop))
        .route("/{id}/uninstall", post(post_uninstall))
        .route("/{id}/trust", post(post_trust))
}

async fn list_apps(State(state): State<ServeState>) -> Response {
    let apps = Arc::clone(&state.apps);
    match tokio::task::spawn_blocking(move || apps.list()).await {
        Ok(list) => ok(list),
        Err(err) => api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("list apps task: {err}"),
        ),
    }
}

async fn get_readme(State(state): State<ServeState>, Path(id): Path<String>) -> Response {
    let apps = Arc::clone(&state.apps);
    let app_id = id.clone();
    match tokio::task::spawn_blocking(move || apps.readme(&app_id)).await {
        Ok(Ok(text)) => ok(ReadmeData { id, text }),
        Ok(Err(err)) => app_err(err),
        Err(err) => api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("readme task: {err}"),
        ),
    }
}

async fn post_install(State(state): State<ServeState>, Path(id): Path<String>) -> Response {
    match state.apps.install(&id).await {
        Ok(()) => ok(AppIdData { id }),
        Err(err) => app_err(err),
    }
}

async fn post_start(State(state): State<ServeState>, Path(id): Path<String>) -> Response {
    match state.apps.start(&id).await {
        Ok(()) => ok(AppIdData { id }),
        Err(err) => app_err(err),
    }
}

async fn post_stop(State(state): State<ServeState>, Path(id): Path<String>) -> Response {
    match state.apps.stop(&id).await {
        Ok(()) => ok(AppIdData { id }),
        Err(err) => app_err(err),
    }
}

async fn post_uninstall(State(state): State<ServeState>, Path(id): Path<String>) -> Response {
    match state.apps.uninstall(&id).await {
        Ok(()) => ok(AppIdData { id }),
        Err(err) => app_err(err),
    }
}

async fn post_trust(
    State(state): State<ServeState>,
    Path(id): Path<String>,
    Json(body): Json<TrustBody>,
) -> Response {
    match state.apps.trust(&id, &body.definition_hash) {
        Ok(()) => ok(AppIdData { id }),
        Err(err) => app_err(err),
    }
}

fn ok<T: Serialize>(data: T) -> Response {
    Json(ApiOk { ok: true, data }).into_response()
}

fn api_err(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ApiErr {
            ok: false,
            error: message.into(),
        }),
    )
        .into_response()
}

fn app_err(err: AppError) -> Response {
    let status = match &err {
        AppError::NotFound => StatusCode::NOT_FOUND,
        AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
        AppError::Forbidden(_) => StatusCode::FORBIDDEN,
        AppError::Conflict(_) => StatusCode::CONFLICT,
        AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let message = err.message().to_string();
    api_err(status, message)
}

/// Used by the router wiring without leaking the service type into callers.
pub fn service(state: &ServeState) -> &Arc<AppService> {
    &state.apps
}
