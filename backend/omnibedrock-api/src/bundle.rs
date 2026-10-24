//! The panel UI, compiled into the binary.
//!
//! The static SPA build (`frontend/build`) is embedded at compile time and
//! served from the same process as the API, so one executable is the whole
//! panel. App routes (`/store`, `/addons`, …) have no file extension and fall
//! back to `index.html`, letting the client router take over.
use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    response::{IntoResponse, Response},
};
use include_dir::{include_dir, Dir};

static UI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../frontend/build");

/// Serve an embedded UI asset, falling back to the SPA shell for app routes.
pub async fn serve(request: Request<Body>) -> Response {
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return (StatusCode::METHOD_NOT_ALLOWED, "method not allowed").into_response();
    }
    let raw = request.uri().path().trim_start_matches('/');
    let path = if raw.is_empty() { "index.html" } else { raw };
    // SPA fallback: extension-less paths are client routes served by the
    // shell; anything that looks like a missing asset is a genuine 404.
    let served = match UI.get_file(path) {
        Some(file) => Some((file, path)),
        None if !path.contains('.') => UI.get_file("index.html").map(|file| (file, "index.html")),
        None => None,
    };
    let Some((file, served_path)) = served else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    // Mime always follows the file actually served — the SPA shell must go
    // out as text/html even when the URL was /store, or Chromium blocks it.
    let immutable = served_path.starts_with("_app/");
    let response = Response::builder()
        .header(
            header::CONTENT_TYPE,
            mime_guess::from_path(served_path).first_or_octet_stream().as_ref(),
        )
        .header(
            header::CACHE_CONTROL,
            if immutable {
                // Vite-hashed assets never change content for a given name.
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            },
        );
    if request.method() == Method::HEAD {
        return response.body(Body::empty()).unwrap_or_else(|_| internal_error());
    }
    response
        .body(Body::from(file.contents()))
        .unwrap_or_else(|_| internal_error())
}

fn internal_error() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "failed to build response").into_response()
}
