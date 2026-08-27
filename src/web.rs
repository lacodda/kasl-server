//! Serving the web UI.
//!
//! The built SPA is compiled into the binary (ADR 0012): a self-hosted install
//! is one file, and the UI can never be from a different build than the API it
//! calls. In development the app is served by Vite, which proxies `/api` here,
//! so this path is what a release does rather than what a developer waits for.
//!
//! The fallback is the whole routing story. A single-page app owns its own
//! paths, so anything the API has not claimed is answered with `index.html`
//! and the browser decides what to draw. Without that, a refresh on `/privacy`
//! would 404 - the server has no such file, and only the app knows the route.

use axum::{
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::Embed;

/// The contents of `frontend/dist`, as of compile time.
///
/// `allow_missing` because the directory is a build output: it is gitignored,
/// and `pnpm build` empties it before writing, so nothing committed can keep
/// it in place. Without this a clean checkout would not compile at all, and a
/// Rust-only contributor would be stopped by a missing Node toolchain.
///
/// A binary built that way is honest about it - every path answers "no web UI
/// was built into this binary" rather than an empty page.
#[derive(Embed)]
#[folder = "frontend/dist"]
#[allow_missing = true]
struct Assets;

/// Answers a request that no API route matched.
///
/// An asset if there is one at that path, `index.html` otherwise.
pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // An empty path is the app's root, which is the document itself.
    if path.is_empty() {
        return index_html();
    }

    match Assets::get(path) {
        Some(file) => {
            // The type is decided at compile time for this exact file, by the
            // same crate that embedded it - not guessed again here from a path
            // that could disagree with what was stored.
            let mime = file.metadata.mimetype().to_string();
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, mime),
                    // Vite fingerprints asset filenames, so a name that exists
                    // never changes content and can be cached hard. The
                    // document itself is not in this branch.
                    (header::CACHE_CONTROL, cache_control(path).to_string()),
                ],
                file.data,
            )
                .into_response()
        }
        // Not an asset: a client-side route, and the app resolves it.
        None => index_html(),
    }
}

/// The application document.
///
/// Answered for every unmatched path, including ones the app will itself treat
/// as unknown - the server cannot tell a mistyped URL from a valid route it
/// has never heard of, and only the app can.
fn index_html() -> Response {
    match Assets::get("index.html") {
        Some(file) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
                // Never cached: it names the fingerprinted bundles, so a stale
                // copy would keep pointing at a build that no longer exists.
                (header::CACHE_CONTROL, "no-cache".to_string()),
            ],
            file.data,
        )
            .into_response(),
        // A binary built without a frontend. Says so plainly rather than
        // answering an empty 200, which reads as "the app is broken".
        None => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "no web UI was built into this binary; the API is at /api/v1",
        )
            .into_response(),
    }
}

/// How long a browser may keep a file.
///
/// Fingerprinted names (`index-X9yN6mCH.js`) are immutable by construction;
/// anything else keeps a short leash because its name says nothing about its
/// contents.
fn cache_control(path: &str) -> &'static str {
    if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=3600"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprinted_bundles_are_cached_forever_and_others_are_not() {
        // The failure this guards is invisible and lasts a year: caching
        // `index.html` immutably would pin a browser to a build that has been
        // replaced, with no way to recover but a hard reload.
        assert_eq!(cache_control("assets/index-X9yN6mCH.js"), "public, max-age=31536000, immutable");
        assert_eq!(cache_control("favicon.svg"), "public, max-age=3600");
        assert_eq!(cache_control("index.html"), "public, max-age=3600");
    }

    /// Whether this binary carries a built frontend at all.
    ///
    /// The suite runs both ways - a developer who has never run `pnpm build`
    /// and CI, which builds it first - so the assertions below say which case
    /// they are in rather than accepting either outcome for the same input.
    fn frontend_was_embedded() -> bool {
        Assets::get("index.html").is_some()
    }

    #[tokio::test]
    async fn a_client_side_route_is_answered_with_the_document() {
        // A refresh on `/privacy` must not 404: the server has no such file,
        // and only the app knows the route.
        let response = serve("/privacy".parse().unwrap()).await;
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();

        if frontend_was_embedded() {
            assert_eq!(response.status(), StatusCode::OK, "a built app must answer its own routes");
            assert_eq!(content_type, "text/html; charset=utf-8");
        } else {
            // Without a frontend there is nothing to answer with, and saying
            // so beats an empty 200 that reads as a broken app.
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert_eq!(content_type, "text/plain; charset=utf-8");
        }
    }

    #[tokio::test]
    async fn the_root_is_the_document_too() {
        let response = serve("/".parse().unwrap()).await;
        let expected = if frontend_was_embedded() { StatusCode::OK } else { StatusCode::NOT_FOUND };
        assert_eq!(response.status(), expected);
    }

    #[tokio::test]
    async fn an_embedded_asset_is_served_with_its_own_type() {
        // Skipped rather than faked when there is no build: asserting on an
        // asset that does not exist would test the fallback a second time.
        let Some(name) = Assets::iter().find(|name| name.ends_with(".js")) else {
            eprintln!("skipped: no frontend build is embedded in this binary");
            return;
        };

        let response = serve(format!("/{name}").parse().unwrap()).await;
        assert_eq!(response.status(), StatusCode::OK);

        let content_type = response.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap();
        assert!(content_type.contains("javascript"), "a bundle served as `{content_type}` will not execute");

        let cache = response.headers().get(header::CACHE_CONTROL).unwrap().to_str().unwrap();
        assert!(cache.contains("immutable"), "a fingerprinted bundle should be cached hard, got `{cache}`");
    }
}
