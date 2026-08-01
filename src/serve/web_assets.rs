use std::sync::LazyLock;

use crate::config::Audience;

const REVIEW_HTML: &str = include_str!("../../assets/web/review.html");
const KIDS_HTML: &str = include_str!("../../assets/web/kids/kids.html");
pub(super) const THEME_CSS: &str = include_str!("../../assets/web/theme.css");
pub(super) const THEME_JS: &str = include_str!("../../assets/web/theme.js");
pub(super) const ALIX_LOGO_JS: &str = include_str!("../../assets/web/alix-logo.js");
const HEAD_HTML: &str = include_str!("../../assets/web/_head.html");
const BRAND_HTML: &str = include_str!("../../assets/web/_brand.html");

const REVIEW_CSS: &str = concat!(
    include_str!("../../assets/web/review/shell.css"),
    "\n",
    include_str!("../../assets/web/review/study.css"),
    "\n",
    include_str!("../../assets/web/review/picker.css"),
    "\n",
    include_str!("../../assets/web/review/ai.css"),
    "\n",
    include_str!("../../assets/web/review/walk.css"),
    "\n",
);
const REVIEW_JS: &str = concat!(
    include_str!("../../assets/web/review/contracts.js"),
    "\n",
    include_str!("../../assets/web/review/api.js"),
    "\n",
    include_str!("../../assets/web/review/model.js"),
    "\n",
    include_str!("../../assets/web/review/dom.js"),
    "\n",
    include_str!("../../assets/web/review/exam.js"),
    "\n",
    include_str!("../../assets/web/review/app.js"),
    "\n",
);

#[cfg(test)]
pub(super) const REVIEW_ASSET_MANIFEST: &str =
    include_str!("../../assets/web/review/manifest.json");
#[cfg(test)]
pub(super) const REVIEW_CSS_SOURCES: &[&str] =
    &["shell.css", "study.css", "picker.css", "ai.css", "walk.css"];
#[cfg(test)]
pub(super) const REVIEW_JS_SOURCES: &[&str] = &[
    "contracts.js",
    "api.js",
    "model.js",
    "dom.js",
    "exam.js",
    "app.js",
];

const PLEX_SANS_400: &[u8] = include_bytes!("../../assets/web/fonts/ibm-plex-sans-400.woff2");
const PLEX_SANS_500: &[u8] = include_bytes!("../../assets/web/fonts/ibm-plex-sans-500.woff2");
const PLEX_SANS_600: &[u8] = include_bytes!("../../assets/web/fonts/ibm-plex-sans-600.woff2");
const PLEX_SANS_700: &[u8] = include_bytes!("../../assets/web/fonts/ibm-plex-sans-700.woff2");
const PLEX_MONO_400: &[u8] = include_bytes!("../../assets/web/fonts/ibm-plex-mono-400.woff2");
const PLEX_MONO_500: &[u8] = include_bytes!("../../assets/web/fonts/ibm-plex-mono-500.woff2");
const PLEX_MONO_600: &[u8] = include_bytes!("../../assets/web/fonts/ibm-plex-mono-600.woff2");
const PLEX_MONO_700: &[u8] = include_bytes!("../../assets/web/fonts/ibm-plex-mono-700.woff2");

const BALOO2_400: &[u8] = include_bytes!("../../assets/web/kids/fonts/baloo2-400.woff2");
const BALOO2_500: &[u8] = include_bytes!("../../assets/web/kids/fonts/baloo2-500.woff2");
const BALOO2_600: &[u8] = include_bytes!("../../assets/web/kids/fonts/baloo2-600.woff2");
const BALOO2_700: &[u8] = include_bytes!("../../assets/web/kids/fonts/baloo2-700.woff2");
const BALOO2_800: &[u8] = include_bytes!("../../assets/web/kids/fonts/baloo2-800.woff2");

static REVIEW_PAGE: LazyLock<String> = LazyLock::new(|| compose_page(REVIEW_HTML));
static KIDS_PAGE: LazyLock<String> = LazyLock::new(|| compose_page(KIDS_HTML));

pub(super) fn adult_asset(path: &str) -> Option<(&'static str, &'static str)> {
    match path {
        "/review.css" => Some((REVIEW_CSS, "text/css; charset=utf-8")),
        "/review.js" => Some((REVIEW_JS, "application/javascript; charset=utf-8")),
        _ => None,
    }
}

pub(super) fn font_bytes(name: &str) -> Option<&'static [u8]> {
    match name {
        "ibm-plex-sans-400.woff2" => Some(PLEX_SANS_400),
        "ibm-plex-sans-500.woff2" => Some(PLEX_SANS_500),
        "ibm-plex-sans-600.woff2" => Some(PLEX_SANS_600),
        "ibm-plex-sans-700.woff2" => Some(PLEX_SANS_700),
        "ibm-plex-mono-400.woff2" => Some(PLEX_MONO_400),
        "ibm-plex-mono-500.woff2" => Some(PLEX_MONO_500),
        "ibm-plex-mono-600.woff2" => Some(PLEX_MONO_600),
        "ibm-plex-mono-700.woff2" => Some(PLEX_MONO_700),
        "baloo2-400.woff2" => Some(BALOO2_400),
        "baloo2-500.woff2" => Some(BALOO2_500),
        "baloo2-600.woff2" => Some(BALOO2_600),
        "baloo2-700.woff2" => Some(BALOO2_700),
        "baloo2-800.woff2" => Some(BALOO2_800),
        _ => None,
    }
}

pub(super) fn app_page(audience: Audience) -> &'static str {
    match audience {
        Audience::Adult => &REVIEW_PAGE,
        Audience::Kids => &KIDS_PAGE,
    }
}

fn compose_page(html: &str) -> String {
    html.replace("<!--%head%-->", HEAD_HTML)
        .replace("<!--%brand%-->", BRAND_HTML)
}
