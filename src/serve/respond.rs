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

pub(super) fn read_delta(request: &mut Request) -> Option<Delta> {
    #[derive(Deserialize)]
    struct Body {
        delta: String,
    }
    let body: Body = serde_json::from_reader(request.as_reader()).ok()?;
    Delta::from_key(body.delta.chars().next()?)
}

pub(super) fn request_path(request: &Request) -> String {
    request.url().split('?').next().unwrap_or("").to_string()
}

// Only /api/* is guarded: the HTML shell/assets/images stay open so the
// browser can bootstrap its token from the URL.
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

// Constant-time so checking the pairing token doesn't leak it through
// timing. Length is not secret, so an early mismatch return is safe.
pub(super) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub(super) fn header_value<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str())
}

pub(super) fn query_param(url: &str, key: &str) -> Option<String> {
    let (_, query) = url.split_once('?')?;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

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

pub(super) fn read_index(request: &mut Request) -> Option<usize> {
    #[derive(Deserialize)]
    struct Body {
        index: usize,
    }
    let body: Body = serde_json::from_reader(request.as_reader()).ok()?;
    Some(body.index)
}

// `take(cap + 1)` lets an oversized body read one byte past the cap, so an
// unbounded/lying reader is still capped by bytes actually read.
pub(super) fn read_capped(reader: impl Read, cap: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    if reader.take(cap as u64 + 1).read_to_end(&mut bytes).is_err() || bytes.len() > cap {
        None
    } else {
        Some(bytes)
    }
}

// no-cache forces revalidation (alix ships no version in its URLs, so a
// stale cache could show an old app after an upgrade); no-store excludes JSON.
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

pub(super) fn respond_asset(request: Request, body: &str, content_type: &str) {
    let header = Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap();
    let _ = request.respond(
        Response::from_string(body.to_string())
            .with_header(header)
            .with_header(cache_header(b"no-cache")),
    );
}

// Unlike respond_bytes: vendored fonts never change shape at a URL, so they
// get a far-future immutable cache (fetched once, ever).
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

// ASCII only (tiny_http header values must be); the alphanumeric check looks
// only at the stem, so a name filtered down to just an extension also falls back.
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

// place_deck's suffix-strip is case-sensitive (else FILE.MD saves as FILE.MD.md);
// slicing 3 bytes off `name` is safe since lowercasing preserves ASCII byte length.
pub(super) fn normalize_md_extension(name: &str, lower_name: &str) -> String {
    if lower_name.ends_with(".md") {
        format!("{}.md", &name[..name.len() - 3])
    } else {
        name.to_string()
    }
}

// Unlike respond_bytes: Content-Disposition: attachment is what makes a
// browser offer save-as, needed for the zip export.
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

pub(super) fn serve_image(request: Request, images: &HashMap<String, PathBuf>, key: &str) {
    match images.get(key) {
        Some(path) => match std::fs::read(path) {
            Ok(bytes) => respond_bytes(request, bytes, content_type(path)),
            Err(_) => respond_status(request, 404),
        },
        None => respond_status(request, 404),
    }
}
