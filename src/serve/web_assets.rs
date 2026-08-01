use std::sync::LazyLock;

use crate::config::Audience;

const REVIEW_HTML: &str = include_str!("../../web/alix/review.html");
const KIDS_HTML: &str = include_str!("../../web/alix-kids/kids.html");
pub(super) const THEME_CSS: &str = include_str!("../../web/shared/theme.css");
pub(super) const THEME_JS: &str = include_str!("../../web/shared/theme.js");
pub(super) const ALIX_LOGO_JS: &str = include_str!("../../web/shared/alix-logo.js");
const HEAD_HTML: &str = include_str!("../../web/shared/_head.html");
const BRAND_HTML: &str = include_str!("../../web/shared/_brand.html");

macro_rules! review_css_sources {
    ($($name:literal => $path:literal),+ $(,)?) => {
        const REVIEW_CSS: &str = concat!($(include_str!($path), "\n",)+);

        #[cfg(test)]
        pub(super) const REVIEW_CSS_SOURCES: &[&str] = &[$($name),+];
    };
}

review_css_sources!(
    "shell.css" => "../../web/alix/review/shell.css",
    "dom.css" => "../../web/alix/review/dom.css",
    "sheets.css" => "../../web/alix/review/sheets.css",
    "study.css" => "../../web/alix/review/study.css",
    "picker.css" => "../../web/alix/review/picker.css",
    "tutor.css" => "../../web/alix/review/tutor.css",
    "exam.css" => "../../web/alix/review/exam.css",
    "augment.css" => "../../web/alix/review/augment.css",
    "walk.css" => "../../web/alix/review/walk.css",
);
const REVIEW_JS: &str = concat!(
    include_str!("../../web/alix/review/contracts.js"),
    "\n",
    include_str!("../../web/alix/review/api.js"),
    "\n",
    include_str!("../../web/alix/review/model.js"),
    "\n",
    include_str!("../../web/alix/review/dom.js"),
    "\n",
    include_str!("../../web/alix/review/picker.js"),
    "\n",
    include_str!("../../web/alix/review/study.js"),
    "\n",
    include_str!("../../web/alix/review/tutor.js"),
    "\n",
    include_str!("../../web/alix/review/exam.js"),
    "\n",
    include_str!("../../web/alix/review/walk.js"),
    "\n",
    include_str!("../../web/alix/review/augment.js"),
    "\n",
    include_str!("../../web/alix/review/sheets.js"),
    "\n",
    include_str!("../../web/alix/review/app.js"),
    "\n",
);

#[cfg(test)]
pub(super) const REVIEW_ASSET_MANIFEST: &str = include_str!("../../web/alix/review/manifest.json");
#[cfg(test)]
pub(super) const REVIEW_JS_SOURCES: &[&str] = &[
    "contracts.js",
    "api.js",
    "model.js",
    "dom.js",
    "picker.js",
    "study.js",
    "tutor.js",
    "exam.js",
    "walk.js",
    "augment.js",
    "sheets.js",
    "app.js",
];

const PLEX_SANS_400: &[u8] = include_bytes!("../../web/shared/fonts/ibm-plex-sans-400.woff2");
const PLEX_SANS_500: &[u8] = include_bytes!("../../web/shared/fonts/ibm-plex-sans-500.woff2");
const PLEX_SANS_600: &[u8] = include_bytes!("../../web/shared/fonts/ibm-plex-sans-600.woff2");
const PLEX_SANS_700: &[u8] = include_bytes!("../../web/shared/fonts/ibm-plex-sans-700.woff2");
const PLEX_MONO_400: &[u8] = include_bytes!("../../web/shared/fonts/ibm-plex-mono-400.woff2");
const PLEX_MONO_500: &[u8] = include_bytes!("../../web/shared/fonts/ibm-plex-mono-500.woff2");
const PLEX_MONO_600: &[u8] = include_bytes!("../../web/shared/fonts/ibm-plex-mono-600.woff2");
const PLEX_MONO_700: &[u8] = include_bytes!("../../web/shared/fonts/ibm-plex-mono-700.woff2");

const BALOO2_400: &[u8] = include_bytes!("../../web/alix-kids/fonts/baloo2-400.woff2");
const BALOO2_500: &[u8] = include_bytes!("../../web/alix-kids/fonts/baloo2-500.woff2");
const BALOO2_600: &[u8] = include_bytes!("../../web/alix-kids/fonts/baloo2-600.woff2");
const BALOO2_700: &[u8] = include_bytes!("../../web/alix-kids/fonts/baloo2-700.woff2");
const BALOO2_800: &[u8] = include_bytes!("../../web/alix-kids/fonts/baloo2-800.woff2");

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
