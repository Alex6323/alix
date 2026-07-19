//! HTTP plumbing: request auth/parsing helpers and response writers. The only
//! file in `serve` that touches `tiny_http` response construction, so every
//! route builds its reply through one of these instead of the crate directly.

use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tiny_http::{Header, Request, Response};

use crate::{
    scheduler::{Grade, keypoint_grade},
    trace::Delta,
};

/// Parses a `{"delta": "g"|"p"|"m"}` self-grade POST body.
pub(super) fn read_delta(request: &mut Request) -> Option<Delta> {
    #[derive(Deserialize)]
    struct Body {
        delta: String,
    }
    let body: Body = serde_json::from_reader(request.as_reader()).ok()?;
    Delta::from_key(body.delta.chars().next()?)
}

/// The path part of a request URL, without any `?query`.
pub(super) fn request_path(request: &Request) -> String {
    request.url().split('?').next().unwrap_or("").to_string()
}

/// Whether a request may proceed. Only `/api/*` is guarded; the HTML shell,
/// theme assets, and images stay open so the browser can bootstrap its token
/// from the `?token=` URL. No token configured (the localhost default) → open.
pub(super) fn is_authorized(
    path: &str,
    auth_header: Option<&str>,
    query_token: Option<&str>,
    token: Option<&str>,
) -> bool {
    let Some(token) = token else { return true };
    if !path.starts_with("/api/") {
        return true;
    }
    let presented = auth_header
        .and_then(|h| h.strip_prefix("Bearer "))
        .or(query_token);
    presented.is_some_and(|p| ct_eq(p.as_bytes(), token.as_bytes()))
}

/// Constant-time byte comparison, so checking the pairing token doesn't leak it
/// through timing. Length is not secret — a length mismatch returns early.
pub(super) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A request header's value by name (case-insensitive), if present.
pub(super) fn header_value<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str())
}

/// A query parameter's value from a full request URL (`/path?k=v&…`).
pub(super) fn query_param(url: &str, key: &str) -> Option<String> {
    let (_, query) = url.split_once('?')?;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

/// The MIME type to serve a card image with, by file extension. Unknown
/// extensions fall back to a generic binary type (the browser still sniffs it).
pub(super) fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// Parses a grade POST body into a [`Grade`]: either an explicit
/// `{"grade":"failed|partly|passed"}`, or `{"covered":n,"total":m}` from the
/// Explain key-point checklist (derived once, in the lib, via `keypoint_grade`).
pub(super) fn read_grade(request: &mut Request) -> Option<Grade> {
    #[derive(Deserialize)]
    struct Body {
        grade: Option<String>,
        covered: Option<usize>,
        total: Option<usize>,
    }
    let body: Body = serde_json::from_reader(request.as_reader()).ok()?;
    if let Some(g) = body.grade.as_deref() {
        return match g {
            "failed" => Some(Grade::Fail),
            "partly" => Some(Grade::Partial),
            "passed" => Some(Grade::Pass),
            _ => None,
        };
    }
    match (body.covered, body.total) {
        (Some(covered), Some(total)) => Some(keypoint_grade(covered, total)),
        _ => None,
    }
}

/// Parses a `{"index": n}` POST body (the browse card to remove).
pub(super) fn read_index(request: &mut Request) -> Option<usize> {
    #[derive(Deserialize)]
    struct Body {
        index: usize,
    }
    let body: Body = serde_json::from_reader(request.as_reader()).ok()?;
    Some(body.index)
}

/// Reads `reader` to end, capped at `cap` bytes: `None` if the read errors or
/// the body exceeds the cap. `take(cap + 1)` lets an oversized body read one
/// byte past the cap, which the length check below catches — so a reader
/// whose declared length lies (or has none) is still bounded by the actual
/// bytes read, not by what it claims.
pub(super) fn read_capped(reader: impl Read, cap: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    if reader.take(cap as u64 + 1).read_to_end(&mut bytes).is_err() || bytes.len() > cap {
        None
    } else {
        Some(bytes)
    }
}

/// The app shell and its assets must never be served stale: alix ships no
/// version in its URLs, so after an upgrade a heuristically-cached page keeps
/// showing the OLD web app (seen in the wild: a week-old review.html surviving
/// a `make install`). `no-cache` forces revalidation on every load — cheap on
/// localhost — and `no-store` keeps live JSON state out of the cache entirely.
pub(super) fn cache_header(policy: &'static [u8]) -> Header {
    Header::from_bytes(&b"Cache-Control"[..], policy).unwrap()
}

pub(super) fn respond_json<T: Serialize>(request: Request, value: &T) {
    let body = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    let header = Header::from_bytes(
        &b"Content-Type"[..],
        &b"application/json; charset=utf-8"[..],
    )
    .unwrap();
    let _ = request.respond(
        Response::from_string(body)
            .with_header(header)
            .with_header(cache_header(b"no-store")),
    );
}

pub(super) fn respond_html(request: Request, html: &str) {
    let header =
        Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap();
    let _ = request.respond(
        Response::from_string(html.to_string())
            .with_header(header)
            .with_header(cache_header(b"no-cache")),
    );
}

/// Serves a static text asset (the shared `theme.css` / `theme.js`) with the
/// given content type.
pub(super) fn respond_asset(request: Request, body: &str, content_type: &str) {
    let header = Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap();
    let _ = request.respond(
        Response::from_string(body.to_string())
            .with_header(header)
            .with_header(cache_header(b"no-cache")),
    );
}

/// Serves a self-hosted webfont file. Unlike `respond_bytes`, the file is
/// vendored (not user content) and never changes shape at a given URL, so it
/// gets a far-future, immutable cache policy — the point of self-hosting is
/// to fetch each font once, ever.
pub(super) fn respond_font(request: Request, bytes: &'static [u8]) {
    let header = Header::from_bytes(&b"Content-Type"[..], &b"font/woff2"[..]).unwrap();
    let _ = request.respond(
        Response::from_data(bytes)
            .with_header(header)
            .with_header(cache_header(b"public, max-age=31536000, immutable")),
    );
}

pub(super) fn respond_status(request: Request, code: u16) {
    let _ = request.respond(Response::from_string(String::new()).with_status_code(code));
}

pub(super) fn respond_bytes(request: Request, bytes: Vec<u8>, content_type: &str) {
    let header = Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap();
    let _ = request.respond(Response::from_data(bytes).with_header(header));
}

/// A Content-Disposition-safe file name: ASCII only (tiny_http header
/// values must be), quotes/backslashes/control characters dropped, and
/// never empty — a fully non-ASCII name falls back to a generic one.
///
/// The alphanumeric check looks only at the stem (before the last `.`):
/// an extension alone (e.g. a non-ASCII name filtered down to `.zip`) is
/// not a real file name, so it also falls back.
pub(super) fn download_filename(name: &str) -> String {
    let safe: String = name
        .chars()
        .filter(|c| c.is_ascii() && !c.is_ascii_control() && *c != '"' && *c != '\\')
        .collect();
    let stem = match safe.rfind('.') {
        Some(idx) => &safe[..idx],
        None => safe.as_str(),
    };
    if !stem.chars().any(|c| c.is_ascii_alphanumeric()) {
        "decks.zip".to_string()
    } else {
        safe
    }
}

/// Normalizes an uploaded name's `.md` extension to lower case before it
/// reaches [`crate::library::place_deck`], whose suffix-strip is
/// case-sensitive (a locked contract) — without this, `FILE.MD` would save
/// as `FILE.MD.md`. `lower_name` is the already-lowercased name, used only
/// to test the ending; slicing 3 bytes off `name` is safe because a matched
/// `.md` ending means the last 3 bytes are that same ASCII extension
/// (lowercasing never changes a string's byte length).
pub(super) fn normalize_md_extension(name: &str, lower_name: &str) -> String {
    if lower_name.ends_with(".md") {
        format!("{}.md", &name[..name.len() - 3])
    } else {
        name.to_string()
    }
}

/// Like [`respond_bytes`], but marks the response as a file to save rather
/// than render inline — the zip export is the one non-JSON API response, and
/// a browser only offers "save as" with `Content-Disposition: attachment`.
pub(super) fn respond_download(
    request: Request,
    bytes: Vec<u8>,
    content_type: &str,
    filename: &str,
) {
    let content_type_header =
        Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap();
    let disposition = format!("attachment; filename=\"{}\"", download_filename(filename));
    let response = Response::from_data(bytes).with_header(content_type_header);
    let _ = match Header::from_bytes(&b"Content-Disposition"[..], disposition.as_bytes()) {
        Ok(disposition_header) => request.respond(response.with_header(disposition_header)),
        Err(_) => request.respond(response),
    };
}

/// Serves the registered image for `key`, or 404 for an unknown key /
/// unreadable file. Shared by the review and browse routes.
pub(super) fn serve_image(request: Request, images: &HashMap<String, PathBuf>, key: &str) {
    match images.get(key) {
        Some(path) => match std::fs::read(path) {
            Ok(bytes) => respond_bytes(request, bytes, content_type(path)),
            Err(_) => respond_status(request, 404),
        },
        None => respond_status(request, 404),
    }
}
