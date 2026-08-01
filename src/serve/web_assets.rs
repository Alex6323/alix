use std::sync::LazyLock;

use crate::config::Audience;

const REVIEW_HTML: &str = include_str!("../../web/alix/review.html");
const KIDS_HTML: &str = include_str!("../../web/alix-kids/kids.html");
pub(super) const THEME_CSS: &str = include_str!("../../web/shared/theme.css");
pub(super) const THEME_JS: &str = include_str!("../../web/shared/theme.js");
pub(super) const ALIX_LOGO_JS: &str = include_str!("../../web/shared/alix-logo.js");
const HEAD_HTML: &str = include_str!("../../web/shared/_head.html");
const BRAND_HTML: &str = include_str!("../../web/shared/_brand.html");

macro_rules! composed_asset_sources {
    ($body:ident, $sources:ident, $($name:literal => $path:literal),+ $(,)?) => {
        const $body: &str = concat!($(include_str!($path), "\n",)+);

        #[cfg(test)]
        pub(super) const $sources: &[&str] = &[$($name),+];
    };
}

composed_asset_sources!(REVIEW_CSS, REVIEW_CSS_SOURCES,
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
composed_asset_sources!(REVIEW_JS, REVIEW_JS_SOURCES,
    "contracts.js" => "../../web/alix/review/contracts.js",
    "api.js" => "../../web/alix/review/api.js",
    "model.js" => "../../web/alix/review/model.js",
    "dom.js" => "../../web/alix/review/dom.js",
    "picker.js" => "../../web/alix/review/picker.js",
    "study.js" => "../../web/alix/review/study.js",
    "tutor.js" => "../../web/alix/review/tutor.js",
    "exam.js" => "../../web/alix/review/exam.js",
    "walk.js" => "../../web/alix/review/walk.js",
    "augment.js" => "../../web/alix/review/augment.js",
    "sheets.js" => "../../web/alix/review/sheets.js",
    "app.js" => "../../web/alix/review/app.js",
);

composed_asset_sources!(KIDS_CSS, KIDS_CSS_SOURCES,
    "shell.css" => "../../web/alix-kids/kids/shell.css",
    "dom.css" => "../../web/alix-kids/kids/dom.css",
    "picker.css" => "../../web/alix-kids/kids/picker.css",
    "study.css" => "../../web/alix-kids/kids/study.css",
    "tutor.css" => "../../web/alix-kids/kids/tutor.css",
    "settings.css" => "../../web/alix-kids/kids/settings.css",
);
composed_asset_sources!(KIDS_JS, KIDS_JS_SOURCES,
    "api.js" => "../../web/alix-kids/kids/api.js",
    "model.js" => "../../web/alix-kids/kids/model.js",
    "dom.js" => "../../web/alix-kids/kids/dom.js",
    "theme.js" => "../../web/alix-kids/kids/theme.js",
    "picker.js" => "../../web/alix-kids/kids/picker.js",
    "study.js" => "../../web/alix-kids/kids/study.js",
    "tutor.js" => "../../web/alix-kids/kids/tutor.js",
    "settings.js" => "../../web/alix-kids/kids/settings.js",
    "app.js" => "../../web/alix-kids/kids/app.js",
);

#[cfg(test)]
pub(super) const REVIEW_ASSET_MANIFEST: &str = include_str!("../../web/alix/review/manifest.json");
#[cfg(test)]
pub(super) const KIDS_ASSET_MANIFEST: &str = include_str!("../../web/alix-kids/kids/manifest.json");

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

pub(super) fn web_asset(path: &str) -> Option<(&'static str, &'static str)> {
    match path {
        "/review.css" => Some((REVIEW_CSS, "text/css; charset=utf-8")),
        "/review.js" => Some((REVIEW_JS, "application/javascript; charset=utf-8")),
        "/kids.css" => Some((KIDS_CSS, "text/css; charset=utf-8")),
        "/kids.js" => Some((KIDS_JS, "application/javascript; charset=utf-8")),
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
