//! The panel UI, compiled into the binary.
//!
//! The static SPA build (`frontend/build`) is embedded at compile time and
//! served from the same process as the API, so one executable is the whole
//! panel. App routes (`/store`, `/addons`, …) have no file extension and fall
//! back to `index.html`, letting the client router take over.
//!
//! Freshness is enforced by `build.rs`: it hashes the embedded tree into
//! [`UI_BUILD_HASH`], and because that hash reaches the crate as a `rustc-env`
//! value, any UI change forces a real recompile — `include_dir!` can never
//! quietly serve a stale dashboard.
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
    response::{IntoResponse, Response},
};
use include_dir::{Dir, include_dir};

/// SHA-256 of the embedded UI tree, injected by build.rs. Printed as
/// `UI build: …` at boot and served from `GET /api/v1/build-id`, so a stale
/// open page (and its operator) can tell exactly which dashboard this
/// backend is running.
pub const UI_BUILD_HASH: &str = env!("OMNI_UI_HASH");

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
    let cache_control = cache_control_for(served_path);
    // The tree hash covers every byte, so hash + path identifies content for
    // as long as this binary runs — and changes wholesale on any UI change.
    let etag = format!(
        "\"{}:{}\"",
        &UI_BUILD_HASH[..16],
        file.path().to_string_lossy()
    );
    if if_none_match_hits(request.headers(), &etag) {
        return Response::builder()
            .header(header::CACHE_CONTROL, cache_control)
            .header(header::ETAG, etag)
            .status(StatusCode::NOT_MODIFIED)
            .body(Body::empty())
            .unwrap_or_else(|_| internal_error());
    }
    let response = Response::builder()
        .header(
            header::CONTENT_TYPE,
            mime_guess::from_path(served_path)
                .first_or_octet_stream()
                .as_ref(),
        )
        .header(header::CACHE_CONTROL, cache_control)
        .header(header::ETAG, etag);
    if request.method() == Method::HEAD {
        return response
            .body(Body::empty())
            .unwrap_or_else(|_| internal_error());
    }
    response
        .body(Body::from(file.contents()))
        .unwrap_or_else(|_| internal_error())
}

/// Cache policy per served file. Only Vite's content-hashed subtree may be
/// cached forever; `/_app/version.json` changes every deploy and must
/// revalidate exactly like the shell.
fn cache_control_for(served_path: &str) -> &'static str {
    if served_path.starts_with("_app/immutable/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    }
}

/// RFC 9110 If-None-Match: `*` matches anything; otherwise the ETag matches
/// when it appears in the client's comma-separated list (compared strongly;
/// the panel never issues weak ETags).
fn if_none_match_hits(headers: &axum::http::HeaderMap, etag: &str) -> bool {
    let Some(value) = headers.get(header::IF_NONE_MATCH) else {
        return false;
    };
    let Ok(raw) = value.to_str() else {
        return false;
    };
    raw.split(',').any(|candidate| {
        let candidate = candidate.trim();
        candidate == "*" || candidate == etag
    })
}

fn internal_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "failed to build response",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    #[test]
    fn immutable_cache_applies_only_to_vite_hashed_assets() {
        assert_eq!(
            cache_control_for("_app/immutable/entry/start.abc123.js"),
            "public, max-age=31536000, immutable"
        );
        // SvelteKit's per-deploy files live under _app/ but outside immutable/.
        assert_eq!(cache_control_for("_app/version.json"), "no-cache");
        assert_eq!(cache_control_for("index.html"), "no-cache");
        assert_eq!(cache_control_for("favicon.png"), "no-cache");
    }

    #[test]
    fn build_hash_contract_holds() {
        // build.rs must inject a full SHA-256 hex digest; every consumer
        // (boot log, build-id route, ETags) slices it.
        assert_eq!(UI_BUILD_HASH.len(), 64);
        assert!(UI_BUILD_HASH.chars().all(|c| c.is_ascii_hexdigit()));
        // The embed is really there and the shell is reachable for the SPA.
        assert!(UI.get_file("index.html").is_some());
    }

    /// Mirror of build.rs's inline-script scan, kept independently in the
    /// test so a scan that drifts from the served policy fails here instead
    /// of blanking the page in a real browser.
    fn inline_script_hashes(html: &str) -> Vec<String> {
        use sha2::{Digest, Sha256};
        let lower = html.to_lowercase();
        let bytes = html.as_bytes();
        let mut hashes = Vec::new();
        let mut cursor = 0;
        while let Some(open_rel) = lower[cursor..].find("<script") {
            let open = cursor + open_rel;
            let Some(tag_end_rel) = lower[open..].find('>') else {
                break;
            };
            let tag_end = open + tag_end_rel;
            let body_start = tag_end + 1;
            let Some(close_rel) = lower[body_start..].find("</script") else {
                break;
            };
            let body_end = body_start + close_rel;
            cursor = body_end;
            if lower[open..tag_end].contains(" src=") {
                continue;
            }
            let mut hasher = Sha256::new();
            hasher.update(&bytes[body_start..body_end]);
            hashes.push(format!("sha256-{}", hex::encode(hasher.finalize())));
        }
        hashes
    }

    #[test]
    fn csp_covers_every_inline_script_in_the_embedded_ui() {
        // THE blank-page regression: the CSP allows the UI's inline scripts
        // only by content hash, so a script whose hash the policy misses is
        // silently blocked and the app never mounts. Recompute every inline
        // script's hash from the embedded index.html and require the served
        // policy to name each one — with no script-src 'unsafe-inline'.
        let index = UI
            .get_file("index.html")
            .expect("embedded UI has index.html");
        let text = std::str::from_utf8(index.contents()).expect("utf-8 index.html");
        let hashes = inline_script_hashes(text);
        assert!(
            !hashes.is_empty(),
            "the real UI ships inline bootstrap scripts; a placeholder tree got embedded"
        );
        let csp = crate::api::content_security_policy();
        for hash in &hashes {
            assert!(
                csp.contains(hash.as_str()),
                "CSP `{csp}` is missing inline-script hash {hash}"
            );
        }
        let script_src = csp
            .split(';')
            .map(str::trim)
            .find(|c| c.starts_with("script-src"))
            .expect("CSP declares script-src");
        assert!(
            !script_src.contains("unsafe-inline"),
            "script-src must stay hash-based, got `{script_src}`"
        );
    }

    #[test]
    fn if_none_match_semantics() {
        let mut headers = HeaderMap::new();
        assert!(!if_none_match_hits(&headers, "\"x\""));
        headers.insert(
            header::IF_NONE_MATCH,
            header::HeaderValue::from_static("\"a\", \"x\""),
        );
        assert!(if_none_match_hits(&headers, "\"x\""));
        assert!(!if_none_match_hits(&headers, "\"y\""));
        headers.insert(header::IF_NONE_MATCH, header::HeaderValue::from_static("*"));
        assert!(if_none_match_hits(&headers, "\"anything\""));
    }
}
