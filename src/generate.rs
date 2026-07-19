use std::{path::PathBuf, sync::mpsc::Receiver};

use anyhow::{Result, bail};

use crate::{
    ask,
    backend::ensure_source_reachable,
    config::{AskConfig, GenerateDeckConfig},
    deck::is_url,
    trace::resolve_source,
};

const DEFAULT_PROMPT: &str = "\
You are an expert at creating spaced-repetition flashcards. Read the web page \
at {url} — use the WebFetch tool to fetch it (once) — and turn its content \
into a flashcard deck.

OUTPUT FORMAT — a Markdown deck, one card after another:
- A card starts with `## ` at column 0, followed by the question/front on the \
same line. Never indent a card front.
- The plain lines BELOW it (no indentation, no bullet) are the answer/back. \
EVERY card MUST have at least one answer line — never write a front with no \
answer. Keep answers short — one fact or a few words; several lines are allowed.
- A `> ` line adds a note shown AFTER answering. Add a note to most cards: a \
brief elaboration, a concrete example, a mnemonic, or why it matters — one or \
two short lines, never just restating the answer. Put each note line on its \
own `> ` line, after the answer lines.
- A fill-in-the-blank (cloze) card hides spans of its answer: wrap each hidden \
span as `\\cloze{...}` inside the answer line. The front is a short \
instruction; the answer line(s) hold the full sentence with the hidden spans \
wrapped. Braces outside a `\\cloze{...}` marker are literal, so code with `{}` \
is fine. The blanks live in the answer lines, NEVER on the front. Use \
`\\cloze{...}` only when there is a natural word to blank out; otherwise write \
a plain question-and-answer card. Example of one plain card followed by one \
cloze card:

## What guarantee does ownership give each value in Rust?
Exactly one owner at a time.
> This is what lets Rust free memory deterministically, with no garbage collector.

## Fill in the ownership rule about scope.
When the owner goes out of scope, the value is \\cloze{dropped}.
> \"Dropped\" means its destructor runs and its memory is freed.

- To start an answer line with a literal `## `, `> `, `---`, `<!--`, or a \
code-fence marker, escape it with a leading backslash (e.g. `\\## `).

Begin the file with exactly this frontmatter block:
---
link: {url}

---
The `link:` key lets the learner ask follow-up questions against the source.

PEDAGOGY — produce a balanced deck of AT MOST {max_cards} cards spread across \
four layers of understanding:
  1. Facts & terminology — definitions and key terms. Prefer cloze (`\\cloze{...}` holes) here.
  2. Concepts & mechanisms — \"why\" and \"how\" questions (plain cards).
  3. Application — \"given X, what happens / what would you do?\" (plain cards).
  4. Connections — how ideas relate, contrast, or build on each other.

CARD QUALITY:
- One idea per card (minimum-information principle); split compound facts.
- The answer must cover exactly what the front asks, no more. If it includes a \
fact the question did not ask for, either narrow the answer to the question, \
widen the question to cover the whole answer, or split into separate cards. \
Extra context goes in the `> ` note, not the answer.
- Do not cram an enumeration into one prose answer. If the answer is a list of \
several items, split it into several one-idea cards instead — one card per item \
or group. Only when the ordered list ITSELF is the thing to learn (steps, a \
sequence) keep it as one card with a `<!-- reveal: line -->` line right below \
its front and one item per answer line.
- A MAPPING of pairs (each X with its Y: ABIs to target triples, terms to \
meanings) is not an ordered sequence. Never author a \"match each X to its Y\" \
card that recalls the whole table at once, and never use `<!-- reveal: line -->` \
for one; make it ONE cloze card, one line per pair with the recalled half in \
`\\cloze{...}`, so every pair is drilled on its own.
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
- Give most cards a `> ` note that adds something beyond the answer (context, \
an example, a caveat, or a memory hook).
- Order cards from foundational to advanced.

REVISE before finishing: re-read the entire draft as a set and merge or delete \
any cards that overlap or test the same idea, so every remaining card is \
distinct. A shorter, non-repetitive deck is better than a long one with \
duplicates.

Output ONLY the final, deduplicated deck text — no markdown code fences, no \
preamble, no closing remarks.";

const DEFAULT_SOURCE_PROMPT: &str = "\
You are an expert at creating spaced-repetition flashcards. Explore the source at \
{source} — your working directory is its root; use the Read, Glob and Grep tools \
(read-only, no write or shell access) — and turn its key facts into a flashcard \
deck.

OUTPUT FORMAT — a Markdown deck, one card after another:
- A card starts with `## ` at column 0, followed by the question/front on the \
same line. Never indent a card front.
- The plain lines BELOW it (no indentation, no bullet) are the answer/back. \
EVERY card MUST have at least one answer line — never write a front with no \
answer. Keep answers short — one fact or a few words; several lines are allowed.
- A `> ` line adds a note shown AFTER answering. Add a note to most cards: a \
brief elaboration, a concrete example, a mnemonic, or why it matters — one or \
two short lines, never just restating the answer.
- A fill-in-the-blank (cloze) card hides spans of its answer: wrap each hidden \
span as `\\cloze{...}` inside the answer line. The front is a short \
instruction; the answer line(s) hold the full sentence with the hidden spans \
wrapped — the blanks live in the answer lines, NEVER on the front. Braces \
outside a `\\cloze{...}` marker are literal, so code with `{}` is fine. Use \
`\\cloze{...}` only when there is a natural word to blank out.
- A `<!-- at: file:start-end -->` line under a card cites where its answer \
lives in the source (e.g. `<!-- at: src/string.rs:120-128 -->`; the path is \
relative to the source root — your working directory). Add one to every card \
whose answer maps to a specific, contiguous range of lines — read the real \
lines, never guess the numbers — so the learner can flip the card to its \
source on reveal. Omit it for a card that synthesizes across several places.
- To start an answer line with a literal `## `, `> `, `---`, `<!--`, or a \
code-fence marker, escape it with a leading backslash (e.g. `\\## `).

Begin the file with exactly this frontmatter block:
---
source: {source}

---
The `source:` key ties the deck to its source, so `alix exam` can later grade \
your understanding against it.

PEDAGOGY — produce a balanced deck of AT MOST {max_cards} cards spread across \
four layers of understanding:
  1. Facts & terminology — definitions and key terms. Prefer cloze (`\\cloze{...}` holes) here.
  2. Concepts & mechanisms — \"why\" and \"how\" questions (plain cards).
  3. Application — \"given X, what happens / what would you do?\" (plain cards).
  4. Connections — how the pieces relate, contrast, or build on each other.

CARD QUALITY:
- One idea per card (minimum-information principle); split compound facts.
- The answer must cover exactly what the front asks, no more. If it includes a \
fact the question did not ask for, either narrow the answer to the question, \
widen the question to cover the whole answer, or split into separate cards. \
Extra context goes in the `> ` note, not the answer.
- Do not cram an enumeration into one prose answer. If the answer is a list of \
several items, split it into several one-idea cards instead — one card per item \
or group. Only when the ordered list ITSELF is the thing to learn (steps, a \
sequence) keep it as one card with a `<!-- reveal: line -->` line right below \
its front and one item per answer line.
- A MAPPING of pairs (each X with its Y: ABIs to target triples, terms to \
meanings) is not an ordered sequence. Never author a \"match each X to its Y\" \
card that recalls the whole table at once, and never use `<!-- reveal: line -->` \
for one; make it ONE cloze card, one line per pair with the recalled half in \
`\\cloze{...}`, so every pair is drilled on its own.
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
- Give most cards a `> ` note that adds something beyond the answer.
- Order cards from foundational to advanced.

REVISE before finishing: re-read the whole draft and merge or delete any cards \
that overlap or test the same idea, so every remaining card is distinct.

Output ONLY the final, deduplicated deck text — no markdown code fences, no \
preamble, no closing remarks.";

const REVIEW_PROMPT: &str = "\
You are reviewing a spaced-repetition flashcard deck for quality, then \
returning the improved deck.

Apply these edits:
- Remove or MERGE cards that test the same fact or overlap heavily — every \
card must test something distinct. This is the most important fix.
- Drop cards that are ambiguous or trivial, or whose `> ` note merely restates \
the answer.
- Tighten any card whose answer covers more than its front asks: narrow the \
answer to the question, move the extra fact to the `> ` note, or split it into \
distinct cards. A front and its answer must ask and tell the same thing.
- Rewrite any card that recalls a whole mapping or table of pairs at once \
(\"match each X to its Y\") as one cloze card: one line per pair, the recalled \
half in `\\cloze{...}`. Ordered steps may stay a `<!-- reveal: line -->` card; \
unordered pairs never.
- Keep the EXACT same file format: the leading `---` frontmatter block, `## ` \
card fronts at column 0, plain answer lines below each front, `> ` notes, and \
any `<!-- key: value -->` directive lines. A cloze card keeps its \
`\\cloze{...}` holes in its answer lines.
- Preserve the good cards and their order; do not invent filler to hit a count.

Output ONLY the improved deck — no commentary, no markdown code fences.

The deck to review:

";

pub fn generate_deck(
    source: &str,
    cfg: &GenerateDeckConfig,
    ask_cfg: &AskConfig,
) -> Result<String> {
    let url = is_url(source);
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

pub fn spawn(
    source: String,
    cfg: GenerateDeckConfig,
    ask: AskConfig,
) -> Receiver<Result<String, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(generate_deck(&source, &cfg, &ask).map_err(|e| format!("{e:#}")));
    });
    rx
}

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
        allowed_tools,
        model: cfg.model.clone().or_else(|| ask_cfg.model.clone()),
        timeout_secs: cfg.timeout_secs,
        cwd,
        source_access: false,
        ..ask_cfg.clone()
    }
}

fn build_review_prompt(deck: &str) -> String {
    format!("{REVIEW_PROMPT}{deck}")
}

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

/// Trailing prose isn't stripped: it can't be told apart from a card's
/// answer line.
fn clean_output(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let Some(start) = lines
        .iter()
        .position(|l| *l == "---" || l.starts_with("# ") || l.starts_with("## "))
    else {
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

fn space_cards(lines: &[&str]) -> String {
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut seen_card = false;
    let mut fence: Option<char> = None;
    for &line in lines {
        match fence {
            Some(ch) => {
                if crate::parser::closes_fence(line, ch) {
                    fence = None;
                }
            }
            None => {
                if let Some(ch) = crate::parser::fence_opener(line) {
                    fence = Some(ch);
                } else if line.starts_with("## ") {
                    if seen_card && out.last().is_some_and(|prev| !prev.trim().is_empty()) {
                        out.push("");
                    }
                    seen_card = true;
                }
            }
        }
        out.push(line);
    }
    out.join("\n")
}

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
    // Only a real path segment loses its extension; the host fallback keeps
    // its dot (it's part of the domain, not an extension).
    let base = match last_segment {
        Some(seg) => seg.rsplit_once('.').map(|(b, _)| b).unwrap_or(seg),
        None => host,
    };

    slugify(base)
}

pub fn deck_name(source: &str) -> String {
    if is_url(source) {
        slug_from_url(source)
    } else {
        slug_from_path(source)
    }
}

pub fn slug_from_path(source: &str) -> String {
    let p = std::path::Path::new(source);
    let base = p
        .file_stem()
        .or_else(|| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("deck");
    slugify(base)
}

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
        assert!(p.contains("link: https://example.org/page"));
        assert!(p.contains("## "));
        assert!(p.contains("four layers"));
        assert!(!p.contains("{url}"));
        assert!(!p.contains("{max_cards}"));
        assert!(p.contains("\\cloze{...}"));
        assert!(p.contains("\\cloze{dropped}"));
        assert!(p.contains("NEVER on the front"));
        assert!(p.contains("never write a front with no answer"));
        assert!(p.contains("Add a note to most cards"));
        assert!(p.contains("Give most cards a `> ` note"));
        assert!(p.contains("NO TWO CARDS MAY TEST THE SAME FACT"));
        assert!(p.contains("REVISE before finishing"));
        assert!(p.contains("cover exactly what the front asks"));
        assert!(p.contains("drilled on its own"));
        assert!(!p.contains("indented answer"));
    }

    #[test]
    fn review_prompt_embeds_the_deck_and_asks_to_dedupe() {
        let p = build_review_prompt("---\nlink: u\n---\n\n## Q\nA\n");
        assert!(p.contains("## Q"));
        assert!(p.contains("MERGE cards that test the same fact"));
        assert!(p.contains("Output ONLY the improved deck"));
        assert!(p.contains("must ask and tell the same thing"));
        assert!(p.contains("one line per pair"));
        assert!(p.ends_with("---\nlink: u\n---\n\n## Q\nA\n"));
        assert!(p.contains("`---` frontmatter block"));
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
        assert!(p.contains("Read, Glob and Grep"));
        assert!(p.contains("source: src/scheduler.rs"));
        assert!(p.contains("AT MOST 8 cards"));
        assert!(!p.contains("WebFetch"));
        assert!(!p.contains("{source}"));
        assert!(p.contains("<!-- at: file:start-end -->"));
        assert!(p.contains("never guess"));
        assert!(p.contains("drilled on its own"));
        assert!(!p.contains("indented answer"));
    }

    #[test]
    fn url_prompt_does_not_ask_for_line_citations() {
        let p = build_prompt("https://example.org/page", true, &cfg(8));
        assert!(!p.contains("<!-- at:"));
    }

    #[test]
    fn slug_from_paths() {
        assert_eq!("scheduler", slug_from_path("src/scheduler.rs"));
        assert_eq!("my-crate", slug_from_path("/home/me/My_Crate"));
    }

    #[test]
    fn clean_strips_code_fence() {
        let raw = "```text\n---\nlink: u\n---\n## Q\nA\n```";
        assert_eq!("---\nlink: u\n---\n## Q\nA", clean_output(raw));
    }

    #[test]
    fn clean_strips_leading_commentary() {
        let raw = "Here is your deck:\n\n---\nlink: u\n---\n## Q\nA\n";
        assert_eq!("---\nlink: u\n---\n## Q\nA", clean_output(raw));
    }

    #[test]
    fn clean_strips_commentary_and_fence_together() {
        let raw = "Here is your deck:\n```text\n---\nlink: u\n---\n## Q\nA\n```";
        assert_eq!("---\nlink: u\n---\n## Q\nA", clean_output(raw));
    }

    #[test]
    fn clean_keeps_a_clean_deck_unchanged() {
        let raw = "## Q\nA";
        assert_eq!("## Q\nA", clean_output(raw));
    }

    #[test]
    fn clean_puts_a_blank_line_between_cards() {
        let raw = "## Q1\nA1\n## Q2\nA2";
        assert_eq!("## Q1\nA1\n\n## Q2\nA2", clean_output(raw));
    }

    #[test]
    fn clean_does_not_double_the_blank_between_cards() {
        let raw = "## Q1\nA1\n\n## Q2\nA2";
        assert_eq!("## Q1\nA1\n\n## Q2\nA2", clean_output(raw));
    }

    #[test]
    fn clean_keeps_the_header_attached_to_the_first_card() {
        let raw = "---\nlink: u\n---\n## Q1\nA1\n## Q2\nA2";
        assert_eq!(
            "---\nlink: u\n---\n## Q1\nA1\n\n## Q2\nA2",
            clean_output(raw)
        );
    }

    #[test]
    fn clean_never_splits_a_fenced_h2_out_of_its_card() {
        let raw = "## Q1\n```\n## not a card\n```\n## Q2\nA2";
        assert_eq!(
            "## Q1\n```\n## not a card\n```\n\n## Q2\nA2",
            clean_output(raw)
        );
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

    #[test]
    fn spawn_delivers_generated_deck_text_on_the_channel() {
        use crate::testutil::{ask_config, exec_lock, fake_reply};

        let _g = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), "---\nlink: https://example.org\n---\n## Q\nA\n");
        let rx = spawn("https://example.org".to_string(), cfg(10), ask_config(&cli));
        match rx.recv().unwrap() {
            Ok(text) => assert!(text.contains("## Q")),
            Err(e) => panic!("generate failed: {e}"),
        }
    }
}
