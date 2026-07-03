//! AI-assisted deck generation.
//!
//! Turns a web page into a flashcard deck by handing its URL and a detailed
//! instruction prompt to the Claude Code CLI (the same runner the ask feature
//! uses). Claude reads the page with the WebFetch tool — already on the ask
//! allowlist — and emits the deck as plain text on stdout; the caller
//! validates and writes it. Claude is never given a write or shell tool, so
//! the safe `dontAsk` + WebFetch/WebSearch permission story is unchanged.

use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::{
    ask,
    backend::ensure_source_reachable,
    config::{AskConfig, GenerateDeckConfig},
    deck::is_url,
    trace::resolve_source,
};

/// The built-in instruction prompt. `{url}` and `{max_cards}` are substituted.
const DEFAULT_PROMPT: &str = "\
You are an expert at creating spaced-repetition flashcards. Read the web page \
at {url} — use the WebFetch tool to fetch it (once) — and turn its content \
into a flashcard deck.

OUTPUT FORMAT — a plain-text deck, one card after another:
- A card starts with `# ` at column 0, followed by the question/front on the \
same line.
- The lines BELOW it, indented (a tab or spaces), are the answer/back. EVERY \
card MUST have at least one indented answer line — never write a front with no \
answer. Keep answers short — one fact or a few words; several lines are allowed.
- An indented `! ` line adds a note shown AFTER answering. Add a note to most \
cards: a brief elaboration, a concrete example, a mnemonic, or why it matters \
— one or two short lines, never just restating the answer. Put each note line \
on its own `! ` line, after the answer lines.
- A fill-in-the-blank (cloze) card starts with `#?` instead of `#`. The `#?` \
line is a short instruction; the INDENTED answer line(s) below it hold the \
full sentence, with each hidden span wrapped in {{double curly braces}}. A lone \
single brace is literal, so code with `{}` is fine in a cloze answer. The \
blanks live in the answer line, NEVER on the `#?` line. Use `#?` only when \
there is a natural word to blank out; otherwise use a plain `#` card. Example \
of one plain card followed by one cloze card:
      # What guarantee does ownership give each value in Rust?
          Exactly one owner at a time.
          ! This is what lets Rust free memory deterministically, with no
          ! garbage collector.
      #? Fill in the ownership rule about scope.
          When the owner goes out of scope, the value is {{dropped}}.
          ! \"Dropped\" means its destructor runs and its memory is freed.
- `% ` lines are comments, ignored by the trainer.
- To begin an answer line with a literal #, %, or !, escape it with a \
backslash: \\#.

Begin the file with exactly these two comment lines:
  % Generated from {url}
  % link: {url}
The `% link:` line lets the learner ask follow-up questions against the source.

PEDAGOGY — produce a balanced deck of AT MOST {max_cards} cards spread across \
four layers of understanding:
  1. Facts & terminology — definitions and key terms. Prefer cloze (#?) here.
  2. Concepts & mechanisms — \"why\" and \"how\" questions (plain cards).
  3. Application — \"given X, what happens / what would you do?\" (plain cards).
  4. Connections — how ideas relate, contrast, or build on each other.

CARD QUALITY:
- One idea per card (minimum-information principle); split compound facts.
- Do not cram an enumeration into one prose answer. If the answer is a list of \
several items, split it into several one-idea cards instead — one card per item \
or group. Only when the ordered list ITSELF is the thing to learn (steps, a \
sequence) keep it as one card with `% mode: line` and one item per indented \
answer line.
- Give answers and notes clean structure when the content has it (short lines, \
one point per line — do NOT prefix items with a bullet or dash; bullets are added \
later by `alix deck augment --target format`); keep an atomic answer atomic — \
never pad a one-word answer into a list.
- Format the question for readability, but never let its layout leak the answer \
(don't hint how many items the answer has).
- NO TWO CARDS MAY TEST THE SAME FACT. If a point is already covered, do not \
add another card for it — vary what each card asks rather than rephrasing the \
same question.
- Fronts must be unambiguous and answerable from memory; avoid yes/no questions.
- Write original questions and answers in your own words; do not copy long \
passages verbatim.
- Give most cards a `! ` note that adds something beyond the answer (context, \
an example, a caveat, or a memory hook).
- Order cards from foundational to advanced.

REVISE before finishing: re-read the entire draft as a set and merge or delete \
any cards that overlap or test the same idea, so every remaining card is \
distinct. A shorter, non-repetitive deck is better than a long one with \
duplicates.

Output ONLY the final, deduplicated deck text — no markdown code fences, no \
preamble, no closing remarks.";

/// The instruction prompt for a **local source** (a file or directory).
/// `{source}` and `{max_cards}` are substituted. Mirrors [`DEFAULT_PROMPT`] but
/// explores the source with read-only file tools and ties the deck to it with a
/// `% source:` line (so `alix exam` can grade against it).
const DEFAULT_SOURCE_PROMPT: &str = "\
You are an expert at creating spaced-repetition flashcards. Explore the source at \
{source} — your working directory is its root; use the Read, Glob and Grep tools \
(read-only, no write or shell access) — and turn its key facts into a flashcard \
deck.

OUTPUT FORMAT — a plain-text deck, one card after another:
- A card starts with `# ` at column 0, followed by the question/front on the \
same line.
- The lines BELOW it, indented (a tab or spaces), are the answer/back. EVERY \
card MUST have at least one indented answer line — never write a front with no \
answer. Keep answers short — one fact or a few words; several lines are allowed.
- An indented `! ` line adds a note shown AFTER answering. Add a note to most \
cards: a brief elaboration, a concrete example, a mnemonic, or why it matters \
— one or two short lines, never just restating the answer.
- A fill-in-the-blank (cloze) card starts with `#?` instead of `#`. The `#?` \
line is a short instruction; the INDENTED answer line(s) below it hold the \
full sentence, with each hidden span wrapped in {{double curly braces}} — the \
blanks live in the answer line, NEVER on the `#?` line. Use `#?` only when there \
is a natural word to blank out.
- A `% at: file:start-end` line indented under a card cites where its answer \
lives in the source (e.g. `% at: src/string.rs:120-128`; the path is relative to \
the source root — your working directory). Add one to every card whose answer \
maps to a specific, contiguous range of lines — read the real lines, never guess \
the numbers — so the learner can flip the card to its source on reveal. Omit it \
for a card that synthesizes across several places.
- `% ` lines are comments, ignored by the trainer. To begin an answer line with \
a literal #, %, or !, escape it with a backslash: \\#.

Begin the file with exactly these two comment lines:
  % Generated from {source}
  % source: {source}
The `% source:` line ties the deck to its source, so `alix exam` can later grade \
your understanding against it.

PEDAGOGY — produce a balanced deck of AT MOST {max_cards} cards spread across \
four layers of understanding:
  1. Facts & terminology — definitions and key terms. Prefer cloze (#?) here.
  2. Concepts & mechanisms — \"why\" and \"how\" questions (plain cards).
  3. Application — \"given X, what happens / what would you do?\" (plain cards).
  4. Connections — how the pieces relate, contrast, or build on each other.

CARD QUALITY:
- One idea per card (minimum-information principle); split compound facts.
- Do not cram an enumeration into one prose answer. If the answer is a list of \
several items, split it into several one-idea cards instead — one card per item \
or group. Only when the ordered list ITSELF is the thing to learn (steps, a \
sequence) keep it as one card with `% mode: line` and one item per indented \
answer line.
- Give answers and notes clean structure when the content has it (short lines, \
one point per line — do NOT prefix items with a bullet or dash; bullets are added \
later by `alix deck augment --target format`); keep an atomic answer atomic — \
never pad a one-word answer into a list.
- Format the question for readability, but never let its layout leak the answer \
(don't hint how many items the answer has).
- NO TWO CARDS MAY TEST THE SAME FACT — vary what each card asks rather than \
rephrasing the same question.
- Ground every card in what the source actually shows; do not invent details \
it doesn't contain. Fronts must be answerable from memory; avoid yes/no questions.
- Give most cards a `! ` note that adds something beyond the answer.
- Order cards from foundational to advanced.

REVISE before finishing: re-read the whole draft and merge or delete any cards \
that overlap or test the same idea, so every remaining card is distinct.

Output ONLY the final, deduplicated deck text — no markdown code fences, no \
preamble, no closing remarks.";

/// The second-pass review prompt; the draft deck is appended to it.
const REVIEW_PROMPT: &str = "\
You are reviewing a spaced-repetition flashcard deck for quality, then \
returning the improved deck.

Apply these edits:
- Remove or MERGE cards that test the same fact or overlap heavily — every \
card must test something distinct. This is the most important fix.
- Drop cards that are ambiguous or trivial, or whose `! ` note merely restates \
the answer.
- Keep the EXACT same file format: the leading `%` comment lines, `# ` and \
`#?` card fronts at column 0, indented answer lines, and indented `! ` notes. \
A `#?` cloze card keeps its blanks ({{like this}}) in its indented answer line.
- Preserve the good cards and their order; do not invent filler to hit a count.

Output ONLY the improved deck — no commentary, no markdown code fences.

The deck to review:

";

/// Generates a deck from `source` (a web page URL **or** a local file/directory
/// path) and returns the cleaned deck text (not yet validated or written). A URL
/// is fetched with WebFetch; a local source is explored read-only at its root.
/// Blocks until the CLI replies or times out.
pub fn generate_deck(
    source: &str,
    cfg: &GenerateDeckConfig,
    ask_cfg: &AskConfig,
) -> Result<String> {
    let url = is_url(source);
    // Gate on the backend's capability before building a prompt or resolving the
    // source: a read-only backend can't fetch a URL, and a future non-file
    // backend can't read a local path.
    ensure_source_reachable(ask_cfg, url)?;
    let cwd = if url {
        None
    } else {
        let (base_dir, _) = resolve_source(None, Some(source));
        Some(base_dir)
    };
    let prompt = build_prompt(source, url, cfg);
    let raw = ask::run(&run_config(cfg, ask_cfg, url, cwd), &prompt, &[])?;
    let deck = clean_output(&raw);
    if deck.trim().is_empty() {
        bail!("the model returned no deck content");
    }
    Ok(deck)
}

/// Runs a separate review pass over a draft `deck` and returns the cleaned,
/// improved deck. A fresh CLI call (no shared session) so the reviewer reads
/// the whole deck with fresh eyes.
pub fn review_deck(deck: &str, cfg: &GenerateDeckConfig, ask_cfg: &AskConfig) -> Result<String> {
    let prompt = build_review_prompt(deck);
    // The reviewer only rewrites the supplied text; no source access needed.
    let raw = ask::run(&run_config(cfg, ask_cfg, true, None), &prompt, &[])?;
    let reviewed = clean_output(&raw);
    if reviewed.trim().is_empty() {
        bail!("the review pass returned no deck content");
    }
    Ok(reviewed)
}

/// The CLI runner config for generation: the ask command/permission with
/// generation's own model and (longer) timeout. A web page keeps the ask
/// allowlist (WebFetch); a local source gets read-only `Read`/`Glob`/`Grep` at
/// its root (`cwd`).
fn run_config(
    cfg: &GenerateDeckConfig,
    ask_cfg: &AskConfig,
    url: bool,
    cwd: Option<PathBuf>,
) -> AskConfig {
    let allowed_tools = if url {
        ask_cfg.allowed_tools.clone()
    } else {
        vec!["Read".to_string(), "Glob".to_string(), "Grep".to_string()]
    };
    AskConfig {
        backend: ask_cfg.backend,
        command: ask_cfg.command.clone(),
        permission_mode: ask_cfg.permission_mode.clone(),
        allowed_tools,
        model: cfg.model.clone().or_else(|| ask_cfg.model.clone()),
        effort: ask_cfg.effort.clone(),
        timeout_secs: cfg.timeout_secs,
        cwd,
        source_access: false,
    }
}

/// Builds the review-pass prompt: the instructions followed by the draft deck.
fn build_review_prompt(deck: &str) -> String {
    format!("{REVIEW_PROMPT}{deck}")
}

/// Fills the prompt template and appends any extra guidance. Picks the web-page
/// template for a URL and the local-source template otherwise; a configured
/// `prompt` override wins for either (`{url}`/`{source}` both resolve to the
/// source).
fn build_prompt(source: &str, url: bool, cfg: &GenerateDeckConfig) -> String {
    let template = cfg.prompt.as_deref().unwrap_or(if url {
        DEFAULT_PROMPT
    } else {
        DEFAULT_SOURCE_PROMPT
    });
    let mut prompt = template
        .replace("{url}", source)
        .replace("{source}", source)
        .replace("{max_cards}", &cfg.max_cards.to_string());
    if let Some(extra) = cfg
        .extra
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        prompt.push_str("\n\nAdditional instructions:\n");
        prompt.push_str(extra);
    }
    prompt
}

/// Strips anything around the deck itself: leading commentary or an opening
/// code fence before the first deck line (a deck always starts with a `%`
/// comment or a `#` card front), and trailing blank or fence lines. Trailing
/// prose can't be told apart from a card's answer line, so the prompt asks for
/// none; what this reliably removes is markdown wrapping and lead-in text.
fn clean_output(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let Some(start) = lines.iter().position(|l| {
        let t = l.trim_start();
        t.starts_with('%') || t.starts_with('#')
    }) else {
        return raw.trim().to_string();
    };
    let mut end = lines.len();
    while end > start + 1 {
        let t = lines[end - 1].trim();
        if t.is_empty() || t.starts_with("```") {
            end -= 1;
        } else {
            break;
        }
    }
    space_cards(&lines[start..end])
}

/// Inserts a blank line before each card front (a `#` at column 0) *after the
/// first*, so a generated deck's cards are visually separated. The first card
/// stays attached to any `%` header above it, and a card already preceded by a
/// blank line is left untouched (no double blanks).
fn space_cards(lines: &[&str]) -> String {
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut seen_card = false;
    for &line in lines {
        if line.starts_with('#') {
            if seen_card && out.last().is_some_and(|prev| !prev.trim().is_empty()) {
                out.push("");
            }
            seen_card = true;
        }
        out.push(line);
    }
    out.join("\n")
}

/// Derives a deck file stem from a URL: the last meaningful path segment
/// (minus query, fragment and extension), slugified; falls back to the host,
/// then `"deck"`.
pub fn slug_from_url(url: &str) -> String {
    let without_scheme = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let (host, path) = match without_scheme.split_once('/') {
        Some((h, p)) => (h, p),
        None => (without_scheme, ""),
    };
    let last_segment = path
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/')
        .rsplit('/')
        .find(|s| !s.is_empty());
    // For a real path segment, drop a file extension; for the host fallback
    // keep it as-is (the dot is part of the domain, not an extension).
    let base = match last_segment {
        Some(seg) => seg.rsplit_once('.').map(|(b, _)| b).unwrap_or(seg),
        None => host,
    };

    slugify(base)
}

/// The default deck file stem for a source: from the URL for a web page, from
/// the path for a local file/directory.
pub fn deck_name(source: &str) -> String {
    if is_url(source) {
        slug_from_url(source)
    } else {
        slug_from_path(source)
    }
}

/// Derives a deck file stem from a local source path: the file stem (or, for a
/// directory, its name), slugified; falls back to `"deck"`.
pub fn slug_from_path(source: &str) -> String {
    let p = std::path::Path::new(source);
    let base = p
        .file_stem()
        .or_else(|| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("deck");
    slugify(base)
}

/// Slugify: lower-case alphanumerics; runs of anything else become a single dash
/// inserted only before the next kept character (so no edge dashes). Empty → `"deck"`.
fn slugify(base: &str) -> String {
    let mut slug = String::new();
    let mut pending_dash = false;
    for c in base.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(c.to_ascii_lowercase());
            pending_dash = false;
        } else {
            pending_dash = true;
        }
    }
    if slug.is_empty() {
        "deck".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(max_cards: usize) -> GenerateDeckConfig {
        GenerateDeckConfig {
            max_cards,
            ..GenerateDeckConfig::default()
        }
    }

    #[test]
    fn prompt_substitutes_url_and_card_count() {
        let p = build_prompt("https://example.org/page", true, &cfg(12));
        assert!(p.contains("https://example.org/page"));
        assert!(p.contains("AT MOST 12 cards"));
        assert!(p.contains("% link: https://example.org/page"));
        // It teaches the format and the four layers.
        assert!(p.contains("#?"));
        assert!(p.contains("four layers"));
        assert!(!p.contains("{url}"));
        assert!(!p.contains("{max_cards}"));
        // Cloze guidance must use double braces and keep the blank in the
        // answer, not on the `#?` line (the bug that broke real generations).
        assert!(p.contains("{{double curly braces}}"));
        assert!(p.contains("{{dropped}}"));
        assert!(p.contains("NEVER on the `#?` line"));
        assert!(p.contains("never write a front with no answer"));
        // Notes must be actively requested, not just described as optional.
        assert!(p.contains("Add a note to most cards"));
        assert!(p.contains("Give most cards a `! ` note"));
        // Always-on self-review against redundancy.
        assert!(p.contains("NO TWO CARDS MAY TEST THE SAME FACT"));
        assert!(p.contains("REVISE before finishing"));
    }

    #[test]
    fn review_prompt_embeds_the_deck_and_asks_to_dedupe() {
        let p = build_review_prompt("% link: u\n# Q\n\tA\n");
        assert!(p.contains("# Q"));
        assert!(p.contains("MERGE cards that test the same fact"));
        assert!(p.contains("Output ONLY the improved deck"));
        assert!(p.ends_with("% link: u\n# Q\n\tA\n"));
    }

    #[test]
    fn extra_guidance_is_appended() {
        let mut g = cfg(10);
        g.extra = Some("Focus on the public API.".to_string());
        let p = build_prompt("u", true, &g);
        assert!(p.contains("Additional instructions:"));
        assert!(p.contains("Focus on the public API."));
    }

    #[test]
    fn full_prompt_override_replaces_template() {
        let mut g = cfg(5);
        g.prompt = Some("Make {max_cards} cards from {url}.".to_string());
        let p = build_prompt("U", true, &g);
        assert_eq!("Make 5 cards from U.", p);
    }

    #[test]
    fn source_prompt_explores_locally_and_ties_to_source() {
        let p = build_prompt("src/scheduler.rs", false, &cfg(8));
        assert!(p.contains("src/scheduler.rs"));
        assert!(p.contains("Read, Glob and Grep")); // read-only file tools
        assert!(p.contains("% source: src/scheduler.rs")); // ties to source for exam
        assert!(p.contains("AT MOST 8 cards"));
        assert!(!p.contains("WebFetch")); // a local source, not a web page
        assert!(!p.contains("{source}"));
        // It asks for per-card `% at:` source citations (read the real lines).
        assert!(p.contains("% at: file:start-end"));
        assert!(p.contains("never guess"));
    }

    #[test]
    fn url_prompt_does_not_ask_for_line_citations() {
        // A web page has no line numbers, so the URL prompt must not request `% at:`.
        let p = build_prompt("https://example.org/page", true, &cfg(8));
        assert!(!p.contains("% at:"));
    }

    #[test]
    fn slug_from_paths() {
        assert_eq!("scheduler", slug_from_path("src/scheduler.rs"));
        assert_eq!("my-crate", slug_from_path("/home/me/My_Crate"));
    }

    #[test]
    fn clean_strips_code_fence() {
        let raw = "```text\n% link: u\n# Q\n\tA\n```";
        assert_eq!("% link: u\n# Q\n\tA", clean_output(raw));
    }

    #[test]
    fn clean_strips_leading_commentary() {
        let raw = "Here is your deck:\n\n% link: u\n# Q\n\tA\n";
        assert_eq!("% link: u\n# Q\n\tA", clean_output(raw));
    }

    #[test]
    fn clean_strips_commentary_and_fence_together() {
        // The realistic case: lead-in line, opening ```text fence, deck, and a
        // closing ``` fence — all of the wrapping must go.
        let raw = "Here is your deck:\n```text\n% link: u\n# Q\n\tA\n```";
        assert_eq!("% link: u\n# Q\n\tA", clean_output(raw));
    }

    #[test]
    fn clean_keeps_a_clean_deck_unchanged() {
        let raw = "# Q\n\tA";
        assert_eq!("# Q\n\tA", clean_output(raw));
    }

    #[test]
    fn clean_puts_a_blank_line_between_cards() {
        let raw = "# Q1\n\tA1\n# Q2\n\tA2";
        assert_eq!("# Q1\n\tA1\n\n# Q2\n\tA2", clean_output(raw));
    }

    #[test]
    fn clean_does_not_double_the_blank_between_cards() {
        let raw = "# Q1\n\tA1\n\n# Q2\n\tA2";
        assert_eq!("# Q1\n\tA1\n\n# Q2\n\tA2", clean_output(raw));
    }

    #[test]
    fn clean_keeps_the_header_attached_to_the_first_card() {
        // The header stays with card 1; only the *second* card gets a blank.
        let raw = "% link: u\n# Q1\n\tA1\n# Q2\n\tA2";
        assert_eq!("% link: u\n# Q1\n\tA1\n\n# Q2\n\tA2", clean_output(raw));
    }

    #[test]
    fn slug_from_typical_urls() {
        assert_eq!(
            "ch04-01-what-is-ownership",
            slug_from_url("https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html")
        );
        assert_eq!(
            "rust-programming-language",
            slug_from_url("https://en.wikipedia.org/wiki/Rust_(programming_language)")
        );
        assert_eq!("example-org", slug_from_url("https://example.org"));
        assert_eq!("page", slug_from_url("https://example.org/page?x=1#frag"));
    }
}
