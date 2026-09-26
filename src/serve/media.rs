//! User-image upload and read (`/api/media`).
//!
//! Bytes land in `.litecode/media/` and are addressed by content hash. The
//! transcript never stores them. `GET` is how the UI paints a thumbnail,
//! because `/api/*` accepts a Bearer header and an `<img src>` cannot send one.

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Serialize;

use crate::serve::state::ServeState;
use crate::session::media::{
    MAX_MEDIA_BLOB_SIZE, media_ref, mime_for_name, read_user_media, sniff_image, valid_media_name,
    write_user_media,
};
use crate::tool::output::image_dimensions;

#[derive(Serialize)]
struct UploadBody {
    ok: bool,
    #[serde(rename = "ref")]
    media_ref: String,
    mime: &'static str,
    width: u32,
    height: u32,
}

#[derive(Serialize)]
struct ErrorBody {
    ok: bool,
    error: String,
}

pub fn router() -> Router<ServeState> {
    Router::new()
        .route("/", post(post_media))
        .route("/{name}", get(get_media))
        .layer(DefaultBodyLimit::max(MAX_MEDIA_BLOB_SIZE as usize))
}

async fn post_media(State(state): State<ServeState>, body: Bytes) -> Response {
    if body.is_empty() {
        return error(StatusCode::BAD_REQUEST, "empty image");
    }
    let Some((mime, ext)) = sniff_image(&body) else {
        return error(StatusCode::BAD_REQUEST, "unsupported image");
    };
    let (width, height) = image_dimensions(&body, mime).unwrap_or((0, 0));
    let data_root = state.sessions.data_root_path();
    match write_user_media(&data_root, &body, ext) {
        Ok(name) => (
            StatusCode::CREATED,
            Json(UploadBody {
                ok: true,
                media_ref: media_ref(&name),
                mime,
                width,
                height,
            }),
        )
            .into_response(),
        Err(error) => error_response(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

async fn get_media(State(state): State<ServeState>, Path(name): Path<String>) -> Response {
    if !valid_media_name(&name) {
        return error(StatusCode::BAD_REQUEST, "invalid image name");
    }
    let data_root = state.sessions.data_root_path();
    match read_user_media(&data_root, &name) {
        Ok(bytes) => {
            let mime = mime_for_name(&name).unwrap_or("application/octet-stream");
            (
                [
                    (header::CONTENT_TYPE, mime),
                    (header::CACHE_CONTROL, "private, no-store"),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    error_response(status, message)
}

fn error_response(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(ErrorBody {
            ok: false,
            error: message.to_string(),
        }),
    )
        .into_response()
}
