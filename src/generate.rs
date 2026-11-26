use std::{
    path::{Path, PathBuf},
    sync::mpsc::Receiver,
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::{
    ask,
    authoring_guide::{Part, part, part_without},
    backend::{ensure_source_reachable, supports_structured_progress},
    choice,
    cite::GeneratedRoot,
    config::{AskConfig, GenerateCardStyle, GenerateDeckConfig},
    deck::is_url,
    parser,
    source::resolve_source,
    sources::commands::TopicSources,
};

pub const DEFAULT_GOAL: &str = "understand the whole source";
pub const RANGED_FILE_LINES: usize = 400;
pub(crate) const SOURCES_MARKER: &str = "=== sources ===";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationSpec {
    pub goal: String,
    pub language: Option<String>,
    pub audience: Option<String>,
    pub card_style: GenerateCardStyle,
    pub diagrams: bool,
}

impl GenerationSpec {
    pub fn from_config(goal: impl Into<String>, config: &GenerateDeckConfig) -> Self {
        Self {
            goal: goal.into(),
            language: config.language.clone(),
            audience: config.audience.clone(),
            card_style: config.card_style,
            diagrams: false,
        }
    }

    pub(crate) fn requirements(&self) -> String {
        self.build_requirements(true)
    }

    pub(crate) fn learner_requirements(&self) -> String {
        self.build_requirements(false)
    }

    fn build_requirements(&self, include_card_style: bool) -> String {
        let mut requirements = format!("GENERATION REQUIREMENTS:\n- Learning goal: {}", self.goal);
        if include_card_style {
            requirements.push_str(&format!("\n- Card style: {}", self.card_style.as_str()));
        }
        if let Some(language) = &self.language {
            requirements.push_str(&format!(
                "\n- Output language: {language}. Write every learner-facing word in this language, including fronts, answers, choices, and notes."
            ));
        }
        if let Some(audience) = &self.audience {
            requirements.push_str(&format!(
                "\n- Audience: {audience}. Match vocabulary, assumed knowledge, examples, and difficulty to this audience."
            ));
        }
        requirements.push_str("\nThese requirements are binding.");
        requirements
    }

    fn is_default(&self) -> bool {
        self.goal == DEFAULT_GOAL
            && self.language.is_none()
            && self.audience.is_none()
            && self.card_style == GenerateCardStyle::Mixed
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FormatConsumer {
    Generation,
    ReviewAndFill,
}

const PRECEDENCE_LINE: &str = "Where the instructions around this guide differ from it (the \
     frontmatter block, topic headings and the sources list, `at:` locators, the card style, \
     the output), those instructions win; a topic heading is a `# ` section and still gets its \
     prose paragraph.";

const SELECT_ALL_LINE: &str = "Where several items of a set share a property and the rest do \
     not, write a select-all card for that property even when a card table already pairs each \
     item with its kind: judging the whole set at once is a different question from recalling \
     one row.";

const CARD_SHAPE_SYNTAX: &str = "\
     - A plain card puts short answer lines below its `## ` front. Do not prefix \
     answers with bullets or dashes.\n\
     - A cloze card has no answer section: write the whole sentence on the `## ` \
     line and hide each span with a `<!-- blank: span hidden=\"...\" -->` comment on \
     its own line directly below it. The hidden text must appear verbatim in that \
     sentence; if it repeats there, add `occurrence=N` (1-based) to pick which one. \
     Never write a separate question above the sentence, an answer line, or `---` on \
     a cloze card. Example:\n\
     ## When the owner leaves scope, the value is dropped.\n\
     <!-- blank: span hidden=\"dropped\" -->\n\
     - An authored-choice card puts 3-5 GitHub task-list options directly below its \
     front: exactly one checked `- [x]` answer and at least two unchecked `- [ ]` \
     distractors, then `<!-- choices: single -->` on the line directly below the last \
     option. Without that line the options stay a literal checklist instead of \
     becoming a card. Add a `> [!NOTE]` note explaining their mistaken premises.\n\
     - A card table starts with `| front | back | note |`, then \
     `| --- | --- | --- |`, then one row per pair, then `<!-- cards -->` on the \
     line DIRECTLY below the last row, with no blank line between. Omit the note \
     column when it is unused. A card table's note is that column: a \
     blockquote below a table does not parse.\n\
     - For ordered steps, write one step per answer line and put `<!-- reveal: line -->` below the \
     last answer line.\n\
     - For a card needed in both directions, put `<!-- direction: both -->` after \
     its answer.\n\
     - For an answer the learner must sketch, put `<!-- input: draw -->` after its \
     answer.";

pub(crate) fn card_format(spec: &GenerationSpec, consumer: FormatConsumer) -> String {
    let rule = style_rule(spec.card_style);
    match consumer {
        FormatConsumer::Generation => {
            let parts = match rule {
                Some(_) => vec![part(Part::Format), part(Part::Practice)],
                None if spec.diagrams => vec![
                    part(Part::Format),
                    part(Part::Shapes),
                    part(Part::Diagrams),
                    part(Part::Practice),
                ],
                None => vec![
                    part(Part::Format),
                    part_without(Part::Shapes, Part::ShapesDiagramRow),
                    part(Part::Practice),
                ],
            };
            let mut text = format!(
                "DECK-AUTHORING GUIDE:\n\n{}\n{PRECEDENCE_LINE}",
                parts.join("\n")
            );
            text.push_str("\n\n");
            text.push_str(rule.unwrap_or(SELECT_ALL_LINE));
            text
        }
        FormatConsumer::ReviewAndFill => match rule {
            Some(rule) => rule.to_string(),
            None => format!(
                "CARD SHAPE GUIDE:\n{}\n{}\nCARD SHAPE SYNTAX:\n{CARD_SHAPE_SYNTAX}",
                part_without(Part::Shapes, Part::ShapesDiagramRow),
                part(Part::CardRules)
            ),
        },
    }
}

fn style_rule(style: GenerateCardStyle) -> Option<&'static str> {
    match style {
        GenerateCardStyle::Mixed => None,
        GenerateCardStyle::Plain => Some(
            "- Write every card as a plain question and answer. The plain unindented lines \
             BELOW the front are the answer/back. EVERY card MUST have at least one short \
             answer line. Do not prefix answers with bullets or dashes. Do not use \
             `blank:` span comments or task-list choices. Split mappings into one card \
             per pair.",
        ),
        GenerateCardStyle::Cloze => Some(
            "- Write EVERY card as cloze. A cloze card has no answer section: write the \
             whole sentence on the `## ` line, and give every hidden span a \
             `<!-- blank: span hidden=\"...\" -->` comment on its own line directly below \
             it. The hidden text must appear verbatim in that sentence; if it repeats, add \
             `occurrence=N` (1-based). Never write a separate question above the sentence, \
             an answer line, or `---`. Do not write plain answers or task-list choices. Put \
             a mapping in one card: a short title on the `## ` line, one pair per line \
             below it, and the blank span comments directly below those lines, each hiding a \
             recalled half. The title must not contain any hidden text.",
        ),
        GenerateCardStyle::AuthoredChoices => Some(
            "- Write EVERY card as authored multiple-choice. Directly below the `## ` front, \
             write 3-5 GitHub task-list options: exactly one checked `- [x]` correct answer \
             and at least two unchecked `- [ ]` plausible distractors. On the line directly \
             below the last option, write `<!-- choices: single -->`; without it the options \
             stay a literal checklist instead of becoming a card. Do not add a separate \
             plain answer or `blank:` span comments. Keep options parallel in form and \
             length. \
             Add a short `> [!NOTE]` note explaining the mistaken premise behind the distractors. \
             For a mapping, make one authored-choice card per pair and use plausible values \
             from the same domain as distractors.",
        ),
    }
}

const DEFAULT_PROMPT: &str = "\
You are an expert at creating spaced-repetition flashcards. Read the web page \
at {url} (use the WebFetch tool to fetch it, once) and turn its content into a \
flashcard deck, aiming for at most {max_cards} cards.

{card_format}

Begin the file with exactly this frontmatter block:
---
link: {url}

---
The `link:` key lets the learner ask follow-up questions against the source.

Output ONLY the final deck text: no markdown code fences, no preamble, no \
closing remarks.";

const DEFAULT_SOURCE_PROMPT: &str = "\
You are an expert at creating spaced-repetition flashcards. Explore the source at \
{source} (your working directory is its root; use the Read, Glob and Grep tools, \
read-only, no write or shell access) and turn its key facts into a flashcard \
deck, aiming for at most {max_cards} cards.

{card_format}

Begin the file with exactly this frontmatter block:
---
choices: single

---

- A `<!-- at: ... -->` line under a card cites where its answer lives in the \
source. Its locator is `{locator}`, e.g. `<!-- at: {locator_example} -->`, where \
`<file>` is relative to the source root (your working directory) and everything \
around it is written exactly as given; a path containing a space is wrapped in \
double quotes, e.g. `{locator_spaced}`. Add one to every card \
whose answer maps to a specific, contiguous range of lines (read the real \
lines, never guess the numbers) so the learner can flip the card to its \
source on reveal. Omit it for a card that synthesizes across several places.

Output ONLY the final deck text: no markdown code fences, no preamble, no \
closing remarks.";

const REVIEW_PROMPT: &str = "\
You are reviewing a spaced-repetition flashcard deck for quality, then \
returning the improved deck.

Apply these edits:
- Remove or MERGE cards that test the same fact or overlap heavily — every \
card must test something distinct. This is the most important fix.
- Drop cards that are ambiguous or trivial, or whose `> [!NOTE]` note merely \
restates the answer.
- Tighten any card whose answer covers more than its front asks: narrow the \
answer to the question, move the extra fact to the `> [!NOTE]` note, or split it into \
distinct cards. A front and its answer must ask and tell the same thing.
{mapping_review}
- Keep the EXACT same file format: the leading `---` frontmatter block, cards \
written as `## ` blocks or card-table rows, plain or task-list answers below \
block fronts, `> [!NOTE]` notes, and any `<!-- key: value -->` directive lines. A \
cloze card keeps its sentence on the `## ` line and its \
`<!-- blank: span hidden=\"...\" -->` comments directly below it, with no answer \
lines.
- Keep every `# ` topic heading exactly as written, in its order: never merge, \
rename, reorder, or drop a topic, and keep each card under its topic.
- Preserve the good cards and their order; do not invent filler to hit a count.

Output ONLY the improved deck — no commentary, no markdown code fences.
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeckReply {
    pub text: String,
    pub sources: Option<String>,
}

pub fn generate_deck(
    source: &str,
    root: Option<&GeneratedRoot>,
    cfg: &GenerateDeckConfig,
    ask_cfg: &AskConfig,
    spec: &GenerationSpec,
) -> Result<DeckReply> {
    let url = is_url(source);
    let root = match (url, root) {
        (true, _) => None,
        (false, Some(root)) => Some(root),
        (false, None) => bail!("a deck from the local source {source} needs a `cite:` root"),
    };
    ensure_source_reachable(ask_cfg, url)?;
    let cwd = if url {
        None
    } else {
        let (base_dir, _) = resolve_source(None, Some(source));
        Some(base_dir)
    };
    let lists_sources = root.is_some();
    let single_file = Path::new(source)
        .is_file()
        .then(|| Path::new(source).file_name())
        .flatten()
        .map(|name| name.to_string_lossy().into_owned());
    let mut prompt = build_prompt(source, root, cfg, spec);
    if lists_sources {
        prompt.push_str(&sources_instruction(single_file.as_deref()));
    }
    let raw = ask::run(&run_config(cfg, ask_cfg, url, cwd), &prompt, &[])?;
    let (deck_part, sources) = if lists_sources {
        split_sources(&raw)
    } else {
        (raw.as_str(), None)
    };
    let deck = clean_output(deck_part);
    if deck.trim().is_empty() {
        bail!("the model returned no deck content");
    }
    validate_card_style(&deck, spec)?;
    Ok(DeckReply {
        text: deck,
        sources: sources.map(str::to_string),
    })
}

fn sources_instruction(single_file: Option<&str>) -> String {
    let only_that_file = single_file.map_or_else(String::new, |name| {
        format!(
            "\n- The deck comes from one file, so every `path` may name only `{name}`; a \
             reply naming any other file is refused."
        )
    });
    format!(
        "\n\nTOPICS AND SOURCES:\n\
         - Group the cards under `# ` topic headings at column 0, one per subject area; \
         every card sits under a topic, and no two topics share a heading.\n\
         - After the last card, write a line `{SOURCES_MARKER}` and then a JSON array \
         naming, for every topic, the files its cards draw on: \
         `[{{\"topic\": \"<the heading exactly as written, without the # >\", \"sources\": \
         [{{\"path\": \"<file relative to the source root>\", \"lines\": \"<ranges>\"}}]}}]`.\n\
         - For a file longer than {RANGED_FILE_LINES} lines, give `lines`: the line ranges \
         the topic's cards rest on, ascending and non-overlapping, e.g. `12-40,88,90-120`. \
         For a shorter file, omit `lines`; the whole file is bound.\n\
         - Name only files you read, never a directory; the learner is examined on them.\
         {only_that_file}"
    )
}

pub(crate) fn split_sources(raw: &str) -> (&str, Option<&str>) {
    let mut offset = 0;
    let mut found = None;
    for line in raw.split_inclusive('\n') {
        if line.trim() == SOURCES_MARKER {
            found = Some((offset, offset + line.len()));
        }
        offset += line.len();
    }
    match found {
        Some((start, end)) => (&raw[..start], Some(&raw[end..])),
        None => (raw, None),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ReturnedTopic {
    topic: String,
    sources: Vec<ReturnedSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ReturnedSource {
    path: String,
    lines: Option<String>,
}

impl ReturnedSource {
    fn resolve(self, root: &Path) -> (PathBuf, Option<String>) {
        let lines = self
            .lines
            .map(|lines| {
                lines
                    .split(',')
                    .map(|part| part.split('-').map(str::trim).collect::<Vec<_>>().join("-"))
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .filter(|lines| !lines.is_empty());
        (root.join(self.path), lines)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedDeck {
    pub text: String,
    pub topic_sources: TopicSources,
}

fn strip_json_fence(text: &str) -> &str {
    let mut body = text.trim();
    if let Some(rest) = body.strip_prefix("```") {
        body = rest.split_once('\n').map_or("", |(_, body)| body);
    }
    body.trim_end().strip_suffix("```").unwrap_or(body).trim()
}

pub(crate) fn parse_source_list(json: &str, root: &Path) -> Result<Vec<(PathBuf, Option<String>)>> {
    let returned: Vec<ReturnedSource> = serde_json::from_str(strip_json_fence(json))
        .context("the model's list of sources is not the JSON array it was asked for")?;
    Ok(returned
        .into_iter()
        .map(|source| source.resolve(root))
        .collect())
}

/// `root` is the folder the model read, which its paths are relative to.
pub fn parse_generated(text: &str, sources: &str, root: &Path) -> Result<GeneratedDeck> {
    let returned: Vec<ReturnedTopic> = serde_json::from_str(strip_json_fence(sources))
        .context("the model's list of sources is not the JSON array it was asked for")?;
    let parsed = parser::parse("generated.md", &parser::normalize(text)).map_err(|error| {
        anyhow::anyhow!(
            "the generated deck does not parse ({error}), so its sources cannot be bound"
        )
    })?;
    let mut topic_sources: TopicSources = Vec::with_capacity(returned.len());
    for topic in returned {
        let heading = topic.topic.trim().trim_start_matches('#').trim();
        let matching: Vec<usize> = parsed
            .topics
            .iter()
            .enumerate()
            .filter(|(_, candidate)| candidate.heading.as_deref() == Some(heading))
            .map(|(index, _)| index)
            .collect();
        let index = match matching.as_slice() {
            [index] => *index,
            [] => {
                bail!("the model bound sources to topic `{heading}`, which the deck does not have")
            }
            _ => bail!("the deck has more than one topic headed `{heading}`"),
        };
        let files = topic
            .sources
            .into_iter()
            .map(|source| source.resolve(root))
            .collect();
        topic_sources.push((index, files));
    }
    check_complete(&parsed, &topic_sources)?;
    Ok(GeneratedDeck {
        text: text.to_string(),
        topic_sources,
    })
}

pub(crate) fn check_complete(
    parsed: &parser::ParsedDeck,
    topic_sources: &TopicSources,
) -> Result<()> {
    if parsed.cards.iter().any(|card| card.topic == 0) {
        bail!(
            "the generated deck has cards before its first `#` topic heading, so they have no \
             topic to bind sources to"
        );
    }
    for (index, topic) in parsed.topics.iter().enumerate().skip(1) {
        if !parsed.cards.iter().any(|card| card.topic == index) {
            continue;
        }
        let heading = topic.heading.as_deref().unwrap_or_default();
        let listed: Vec<&Vec<(PathBuf, Option<String>)>> = topic_sources
            .iter()
            .filter(|(at, _)| *at == index)
            .map(|(_, files)| files)
            .collect();
        match listed.as_slice() {
            [files] if !files.is_empty() => {}
            [] | [_] => bail!("the model listed no sources for topic `{heading}`"),
            _ => bail!("the model listed topic `{heading}` more than once"),
        }
    }
    Ok(())
}

pub fn generated_sources(
    source: &str,
    root: &GeneratedRoot,
    reply: &DeckReply,
    text: &str,
) -> Result<(GeneratedDeck, (String, PathBuf))> {
    let input = Path::new(source);
    let Some(sources) = &reply.sources else {
        bail!("the model returned no `{SOURCES_MARKER}` list of the files it drew on");
    };
    let (input_dir, deck) = if input.is_file() {
        let input_dir = input.parent().unwrap_or(input);
        let deck = parse_generated(text, sources, input_dir)?;
        bind_only_the_source_file(&deck, input)?;
        (input_dir, deck)
    } else {
        (input, parse_generated(text, sources, input)?)
    };
    let depth = root.path_prefix.matches('/').count();
    let Some(folder) = input_dir.ancestors().nth(depth) else {
        bail!(
            "`{}` lies above its `{}` root",
            input_dir.display(),
            root.key
        );
    };
    Ok((deck, (root.key.clone(), folder.to_path_buf())))
}

fn bind_only_the_source_file(deck: &GeneratedDeck, file: &Path) -> Result<()> {
    let source = std::fs::canonicalize(file)
        .with_context(|| format!("cannot resolve the source file `{}`", file.display()))?;
    for (listed, _) in deck.topic_sources.iter().flat_map(|(_, files)| files) {
        if std::fs::canonicalize(listed).ok().as_ref() != Some(&source) {
            bail!(
                "the model listed `{}`, but a deck generated from one file may bind only `{}`",
                listed.display(),
                file.display()
            );
        }
    }
    Ok(())
}

pub fn diagrams_drawn_in(dir: &Path) -> bool {
    crate::workspace::root_for_member_dir(dir).is_some()
        && crate::doctor::command_found(crate::diagram::COMMAND)
}

pub fn spawn(
    source: String,
    cfg: GenerateDeckConfig,
    ask: AskConfig,
) -> Receiver<Result<String, String>> {
    let spec = GenerationSpec::from_config(DEFAULT_GOAL, &cfg);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(
            generate_deck(&source, None, &cfg, &ask, &spec)
                .map(|reply| reply.text)
                .map_err(|e| format!("{e:#}")),
        );
    });
    rx
}

pub fn review_deck(
    deck: &str,
    sources: Option<&str>,
    cfg: &GenerateDeckConfig,
    ask_cfg: &AskConfig,
    spec: &GenerationSpec,
) -> Result<DeckReply> {
    let mut prompt = build_review_prompt(deck, spec);
    if let Some(sources) = sources {
        prompt.push_str(&format!(
            "\n\nTHE DRAFT'S SOURCES, per topic:\n{}\n\nAfter the last card of the \
             improved deck, write a line `{SOURCES_MARKER}` and then the same JSON array \
             for the deck you return: for every topic, the files its cards now draw on, \
             taken from the draft's list, with `lines` kept where given.",
            sources.trim()
        ));
    }
    // The reviewer only rewrites the supplied text; no source access needed.
    let raw = ask::run(&run_config(cfg, ask_cfg, true, None), &prompt, &[])?;
    let (deck_part, listed) = if sources.is_some() {
        split_sources(&raw)
    } else {
        (raw.as_str(), None)
    };
    let reviewed = clean_output(deck_part);
    if reviewed.trim().is_empty() {
        bail!("the review pass returned no deck content");
    }
    validate_card_style(&reviewed, spec)?;
    Ok(DeckReply {
        text: reviewed,
        sources: listed.map(str::to_string),
    })
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
        timeout_secs: cfg.timeout_for(supports_structured_progress(ask_cfg)),
        progress: true,
        idle_timeout_secs: cfg.idle_timeout(),
        cwd,
        source_access: false,
        ..ask_cfg.clone()
    }
}

fn build_review_prompt(deck: &str, spec: &GenerationSpec) -> String {
    let review = REVIEW_PROMPT.replace("{mapping_review}", review_mapping(spec.card_style));
    let card_format = card_format(spec, FormatConsumer::ReviewAndFill);
    format!(
        "{review}{}\n\nNOTE SAFETY:\n- {}\n\n{}\n\nThe deck to review:\n\n{deck}",
        spec.requirements(),
        choice::NOTE_POSITION_INSTRUCTION,
        card_format
    )
}

fn review_mapping(style: GenerateCardStyle) -> &'static str {
    match style {
        GenerateCardStyle::Mixed => {
            "- Rewrite any card that recalls a whole mapping of pairs at once \
             (\"match each X to its Y\") as a card table with one row per pair. Ordered \
             steps may stay a `<!-- reveal: line -->` card."
        }
        GenerateCardStyle::Cloze => {
            "- Rewrite any card that recalls a whole mapping or table of pairs at once \
             (\"match each X to its Y\") as one cloze card: a short title on the `## ` line, one line per pair below it, \
             and blank span comments directly below those lines, each hiding a recalled \
             half. Ordered steps may stay a \
             `<!-- reveal: line -->` card; unordered pairs never."
        }
        GenerateCardStyle::Plain => {
            "- Rewrite any card that recalls a whole mapping or table of pairs at once \
             (\"match each X to its Y\") as distinct plain cards, one pair per card. \
             Ordered steps may stay a `<!-- reveal: line -->` card; unordered pairs never."
        }
        GenerateCardStyle::AuthoredChoices => {
            "- Rewrite any card that recalls a whole mapping or table of pairs at once \
             (\"match each X to its Y\") as distinct authored-choice cards, one pair per \
             card, with plausible values from the same domain as distractors. Ordered \
             steps may stay a `<!-- reveal: line -->` card; unordered pairs never."
        }
    }
}

fn build_prompt(
    source: &str,
    root: Option<&GeneratedRoot>,
    cfg: &GenerateDeckConfig,
    spec: &GenerationSpec,
) -> String {
    let template = cfg.prompt.as_deref().unwrap_or(match root {
        None => DEFAULT_PROMPT,
        Some(_) => DEFAULT_SOURCE_PROMPT,
    });
    let card_format = card_format(spec, FormatConsumer::Generation);
    let has_card_format = template.contains("{card_format}");
    let locator = |file: &str, range: &str| root.map(|root| root.locator(file, range));
    let mut prompt = template
        .replace("{url}", source)
        .replace("{source}", source)
        .replace(
            "{locator}",
            &locator("<file>", "start-end").unwrap_or_default(),
        )
        .replace(
            "{locator_example}",
            &locator("src/string.rs", "120-128").unwrap_or_default(),
        )
        .replace(
            "{locator_spaced}",
            &locator("my notes.md", "3-4").unwrap_or_default(),
        )
        .replace("{max_cards}", &cfg.max_cards.to_string())
        .replace("{card_format}", &card_format);
    if let Some(extra) = cfg
        .extra
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        prompt.push_str("\n\nAdditional instructions:\n");
        prompt.push_str(extra);
    }
    if cfg.prompt.is_none() || !spec.is_default() {
        prompt.push_str("\n\n");
        prompt.push_str(&spec.requirements());
        if !has_card_format {
            prompt.push_str("\n\n");
            prompt.push_str(&card_format);
        }
    }
    prompt.push_str("\n\nNOTE SAFETY:\n- ");
    prompt.push_str(choice::NOTE_POSITION_INSTRUCTION);
    prompt
}

pub fn declare_root(text: &str, root: &GeneratedRoot) -> Result<String> {
    let Some(value) = &root.declare else {
        return Ok(text.to_string());
    };
    let Ok(parsed) = parser::parse("generated.md", text) else {
        return Ok(text.to_string());
    };
    if parsed.frontmatter_span.is_none() {
        return Ok(format!(
            "---\ncite:\n  - {}: {}\n---\n\n{text}",
            root.key,
            parser::yaml_quote(value)
        ));
    }
    if parsed
        .frontmatter
        .cite
        .iter()
        .any(|(key, declared)| key == &root.key && declared == value)
    {
        return Ok(text.to_string());
    }
    Ok(crate::deck::with_added_cite(text, &root.key, value)?)
}

pub(crate) fn validate_card_style(deck: &str, spec: &GenerationSpec) -> Result<()> {
    let parsed = match parser::parse("generated.md", deck) {
        Ok(parsed) => parsed,
        Err(_) if spec.card_style == GenerateCardStyle::Mixed => return Ok(()),
        Err(error) => {
            return Err(anyhow::anyhow!(
                "cannot verify generated card style: {error}"
            ));
        }
    };
    if parsed
        .lints
        .iter()
        .any(|lint| lint.kind == parser::LintKind::ChoiceNoteNamesPosition)
    {
        bail!(
            "the model returned an authored-choice note that names an option by position; \
             options are shuffled, so name the claim or mistaken premise instead"
        );
    }
    if spec.card_style == GenerateCardStyle::Mixed {
        return Ok(());
    }
    let cards = parsed.cards;
    if cards.is_empty() {
        bail!("the model returned no cards");
    }
    let invalid = cards
        .iter()
        .filter(|card| match spec.card_style {
            GenerateCardStyle::Mixed => false,
            GenerateCardStyle::Plain => {
                card.is_blank_card() || !card.authored_distractors.is_empty()
            }
            GenerateCardStyle::Cloze => !card.is_blank_card(),
            GenerateCardStyle::AuthoredChoices => {
                !(2..=4).contains(&card.authored_distractors.len())
            }
        })
        .count();
    if invalid > 0 {
        bail!(
            "the model returned {invalid} card(s) that do not match the requested {} style",
            match spec.card_style {
                GenerateCardStyle::AuthoredChoices => "authored multiple-choice",
                style => style.as_str(),
            }
        );
    }
    Ok(())
}

/// Trailing prose isn't stripped: it can't be told apart from a card's
/// answer line.
fn clean_output(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let Some(start) = lines.iter().position(|l| {
        crate::parser::is_frontmatter_fence(l) || l.starts_with("# ") || l.starts_with("## ")
    }) else {
        return raw.trim().to_string();
    };
    let body = &lines[start..];
    let end = match unclosed_fence_opener(body) {
        Some(opener) if body[opener + 1..].iter().all(|line| line.trim().is_empty()) => opener,
        _ => body.len(),
    };
    let last_kept = body[..end]
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .unwrap_or(0);
    space_cards(&body[..=last_kept])
}

fn unclosed_fence_opener(lines: &[&str]) -> Option<usize> {
    let mut open: Option<(usize, char, usize)> = None;
    for (index, &line) in lines.iter().enumerate() {
        match open {
            Some((_, ch, width)) => {
                if crate::parser::closes_fence(line, ch, width) {
                    open = None;
                }
            }
            None => {
                open = crate::parser::fence_opener(line).map(|(ch, width)| (index, ch, width));
            }
        }
    }
    open.map(|(index, _, _)| index)
}

fn space_cards(lines: &[&str]) -> String {
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut seen_card = false;
    let mut fence: Option<(char, usize)> = None;
    for &line in lines {
        match fence {
            Some((ch, open)) => {
                if crate::parser::closes_fence(line, ch, open) {
                    fence = None;
                }
            }
            None => {
                if let Some(opened) = crate::parser::fence_opener(line) {
                    fence = Some(opened);
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn cfg(max_cards: usize) -> GenerateDeckConfig {
        GenerateDeckConfig {
            max_cards,
            ..GenerateDeckConfig::default()
        }
    }

    fn src_root() -> GeneratedRoot {
        GeneratedRoot {
            key: "src".into(),
            path_prefix: String::new(),
            declare: None,
        }
    }

    fn spec() -> GenerationSpec {
        GenerationSpec::from_config(DEFAULT_GOAL, &GenerateDeckConfig::default())
    }

    const STYLES: [GenerateCardStyle; 4] = [
        GenerateCardStyle::Mixed,
        GenerateCardStyle::Plain,
        GenerateCardStyle::Cloze,
        GenerateCardStyle::AuthoredChoices,
    ];

    fn styled(card_style: GenerateCardStyle) -> GenerationSpec {
        GenerationSpec {
            card_style,
            ..spec()
        }
    }

    fn fill(spec: &GenerationSpec) -> String {
        crate::explore::fill_prompt(
            &[crate::explore::Item {
                num: 1,
                kind: crate::explore::Kind::Deck,
                title: "Facts".to_string(),
                requires: Vec::new(),
                source: "notes.md".to_string(),
            }],
            spec,
        )
    }

    fn whole_lines(text: &str) -> std::collections::HashSet<&str> {
        text.lines().map(str::trim_end).collect()
    }

    fn lines_unique_to(texts: &[String], excluded: &[String]) -> Vec<String> {
        let excluded: std::collections::HashSet<&str> =
            excluded.iter().flat_map(|text| whole_lines(text)).collect();
        texts
            .iter()
            .flat_map(|text| text.lines().map(str::trim_end))
            .filter(|line| !line.is_empty() && !excluded.contains(line))
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn run_config_prefers_the_generate_model_and_falls_back_to_the_ask_model() {
        let ask = AskConfig {
            model: Some("ask-model".to_string()),
            ..AskConfig::default()
        };

        let mut generate = cfg(10);
        generate.model = Some("deck-model".to_string());
        assert_eq!(
            Some("deck-model".to_string()),
            run_config(&generate, &ask, true, None).model,
            "the generate-scoped model must win over the ask model"
        );

        generate.model = None;
        assert_eq!(
            Some("ask-model".to_string()),
            run_config(&generate, &ask, true, None).model,
            "without a generate model the ask model applies"
        );
    }

    #[test]
    fn generation_spec_from_config_leaves_diagrams_off() {
        assert!(!spec().diagrams);
    }

    #[test]
    fn the_diagrams_switch_reaches_only_the_mixed_generation_prompt() {
        let source_root = src_root();
        for card_style in STYLES {
            let off = styled(card_style);
            let on = GenerationSpec {
                diagrams: true,
                ..styled(card_style)
            };
            let style = card_style.as_str();
            for (name, root) in [("a url", None), ("a source root", Some(&source_root))] {
                let prompt_off = build_prompt("notes", root, &cfg(10), &off);
                let prompt_on = build_prompt("notes", root, &cfg(10), &on);
                if card_style == GenerateCardStyle::Mixed {
                    assert_ne!(
                        prompt_off, prompt_on,
                        "{style} generation prompt for {name} ignores the diagrams switch"
                    );
                    assert!(
                        prompt_on.contains(&part(Part::Diagrams)),
                        "{style} generation prompt for {name}, diagrams on, lacks the diagrams part"
                    );
                    assert!(
                        !prompt_off.contains(&part(Part::Diagrams)),
                        "{style} generation prompt for {name}, diagrams off, carries the diagrams part"
                    );
                } else {
                    assert_eq!(
                        prompt_off, prompt_on,
                        "{style} generation prompt for {name} follows the diagrams switch"
                    );
                }
            }
            assert_eq!(
                build_review_prompt("## Q\nA\n", &off),
                build_review_prompt("## Q\nA\n", &on),
                "{style} review prompt follows the diagrams switch"
            );
            assert_eq!(
                off.requirements(),
                on.requirements(),
                "{style} requirements"
            );
            assert_eq!(
                off.learner_requirements(),
                on.learner_requirements(),
                "{style} learner requirements"
            );
        }
    }

    #[test]
    fn the_mixed_generation_prompt_carries_format_shapes_and_practice_in_order_and_never_workflow_or_sources()
     {
        let never = lines_unique_to(
            &[part(Part::Workflow), part(Part::Sources)],
            &[
                part(Part::Format),
                part(Part::Shapes),
                part(Part::Diagrams),
                part(Part::Practice),
            ],
        );
        assert!(
            !never.is_empty(),
            "the workflow and sources parts have no line of their own"
        );
        let source_root = src_root();
        for diagrams in [true, false] {
            let carried = if diagrams {
                vec![
                    ("format", part(Part::Format)),
                    ("shapes", part(Part::Shapes)),
                    ("diagrams", part(Part::Diagrams)),
                    ("practice", part(Part::Practice)),
                ]
            } else {
                vec![
                    ("format", part(Part::Format)),
                    (
                        "shapes without the diagram row",
                        part_without(Part::Shapes, Part::ShapesDiagramRow),
                    ),
                    ("practice", part(Part::Practice)),
                ]
            };
            for (name, root) in [("a url", None), ("a source root", Some(&source_root))] {
                let prompt = build_prompt(
                    "notes",
                    root,
                    &cfg(12),
                    &GenerationSpec { diagrams, ..spec() },
                );
                let mut from = 0;
                for (part_name, text) in &carried {
                    let found = prompt[from..].find(text.as_str());
                    assert!(
                        found.is_some(),
                        "Mixed generation prompt for {name}, diagrams {diagrams}: the {part_name} \
                         part is missing or out of order: {prompt}"
                    );
                    from += found.unwrap_or_default() + text.len();
                }
                let lines = whole_lines(&prompt);
                for line in &never {
                    assert!(
                        !lines.contains(line.as_str()),
                        "Mixed generation prompt for {name}, diagrams {diagrams}: carries the \
                         workflow or sources line {line:?}"
                    );
                }
                if !diagrams {
                    assert!(
                        !prompt.contains("A process that branches"),
                        "Mixed generation prompt for {name}, diagrams off: carries the diagram row"
                    );
                    assert!(
                        !prompt.contains(&part(Part::Diagrams)),
                        "Mixed generation prompt for {name}, diagrams off: carries the diagrams part"
                    );
                    for row in [
                        "| Ordered steps in one straight line",
                        "| An answer that cannot be typed",
                    ] {
                        assert!(
                            prompt.contains(row),
                            "Mixed generation prompt for {name}, diagrams off: lost the row {row:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_select_all_line_reaches_mixed_generation_prompts_only() {
        let source_root = src_root();
        for card_style in STYLES {
            let style = card_style.as_str();
            let mixed = card_style == GenerateCardStyle::Mixed;
            for (name, root) in [("a url", None), ("a source root", Some(&source_root))] {
                assert_eq!(
                    build_prompt("notes", root, &cfg(12), &styled(card_style))
                        .contains(SELECT_ALL_LINE),
                    mixed,
                    "{style} generation prompt for {name}: select-all line present must be {mixed}"
                );
            }
            assert!(
                !build_review_prompt("## Q\nA\n", &styled(card_style)).contains(SELECT_ALL_LINE),
                "{style} review prompt carries the select-all line"
            );
            assert!(
                !fill(&styled(card_style)).contains(SELECT_ALL_LINE),
                "{style} fill prompt carries the select-all line"
            );
        }
    }

    #[test]
    fn the_precedence_line_reaches_every_generation_prompt_and_never_review_or_fill() {
        let source_root = src_root();
        for card_style in STYLES {
            let style = card_style.as_str();
            for (name, root) in [("a url", None), ("a source root", Some(&source_root))] {
                assert!(
                    build_prompt("notes", root, &cfg(12), &styled(card_style))
                        .contains(PRECEDENCE_LINE),
                    "{style} generation prompt for {name} lacks the precedence line"
                );
            }
            assert!(
                !build_review_prompt("## Q\nA\n", &styled(card_style)).contains(PRECEDENCE_LINE),
                "{style} review prompt carries the precedence line"
            );
            assert!(
                !fill(&styled(card_style)).contains(PRECEDENCE_LINE),
                "{style} fill prompt carries the precedence line"
            );
        }
        let custom = |template: &str| GenerateDeckConfig {
            prompt: Some(template.to_string()),
            ..cfg(5)
        };
        let with_placeholder = custom("Cards from {url}.\n\n{card_format}\n");
        let without_placeholder = custom("Cards from {url}.");
        let plain = styled(GenerateCardStyle::Plain);
        for (case, config, spec, expected) in [
            (
                "a custom template with the placeholder",
                &with_placeholder,
                spec(),
                true,
            ),
            (
                "a custom template without the placeholder, non-default spec",
                &without_placeholder,
                plain,
                true,
            ),
            (
                "a custom template without the placeholder, default spec",
                &without_placeholder,
                spec(),
                false,
            ),
        ] {
            assert_eq!(
                expected,
                build_prompt("u", None, config, &spec).contains(PRECEDENCE_LINE),
                "{case}: precedence line expected {expected}"
            );
        }
    }

    #[test]
    fn review_and_fill_carry_shapes_card_rules_and_the_syntax_block_and_nothing_else_of_the_guide()
    {
        let shapes = part_without(Part::Shapes, Part::ShapesDiagramRow);
        let card_rules = part(Part::CardRules);
        let format_outside_card_rules = part(Part::Format).replacen(&card_rules, "", 1);
        let never = lines_unique_to(
            &[
                format_outside_card_rules,
                part(Part::Diagrams),
                part(Part::Practice),
            ],
            &[
                shapes.clone(),
                card_rules.clone(),
                CARD_SHAPE_SYNTAX.to_string(),
            ],
        );
        assert!(
            !never.is_empty(),
            "format, diagrams and practice have no line of their own"
        );
        let mixed = spec();
        let fill_prompt = fill(&mixed);
        for (consumer, prompt) in [
            ("review", build_review_prompt("## Q\nA\n", &mixed)),
            ("fill", fill_prompt.clone()),
        ] {
            for (component, text) in [
                ("shapes without the diagram row", shapes.as_str()),
                ("card rules", card_rules.as_str()),
                ("the syntax block", CARD_SHAPE_SYNTAX),
            ] {
                assert!(
                    prompt.contains(text),
                    "Mixed {consumer} prompt lacks {component}: {prompt}"
                );
            }
            let lines = whole_lines(&prompt);
            for line in &never {
                assert!(
                    !lines.contains(line.as_str()),
                    "Mixed {consumer} prompt carries the guide line {line:?}"
                );
            }
            assert!(
                !prompt.contains(PRECEDENCE_LINE),
                "Mixed {consumer} prompt carries the precedence line"
            );
        }
        assert!(
            whole_lines(&fill_prompt)
                .contains("  required; a blockquote without one is a quotation and belongs to the"),
            "Mixed fill prompt lacks the note-badge rule"
        );
        for card_style in STYLES
            .into_iter()
            .filter(|style| *style != GenerateCardStyle::Mixed)
        {
            let style = card_style.as_str();
            let rule = style_rule(card_style).unwrap_or_default();
            assert_eq!(
                rule,
                card_format(&styled(card_style), FormatConsumer::ReviewAndFill),
                "{style} review and fill text is not exactly its style rule"
            );
            for (consumer, prompt) in [
                (
                    "review",
                    build_review_prompt("## Q\nA\n", &styled(card_style)),
                ),
                ("fill", fill(&styled(card_style))),
            ] {
                assert!(
                    prompt.contains(rule),
                    "{style} {consumer} prompt lacks its style rule"
                );
                for guide_part in [Part::Shapes, Part::CardRules] {
                    assert!(
                        !prompt.contains(part(guide_part).lines().next().unwrap_or_default()),
                        "{style} {consumer} prompt carries the {guide_part:?} part"
                    );
                }
            }
        }
    }

    #[test]
    fn the_intro_line_is_gone_from_every_consumer() {
        let source_root = src_root();
        for card_style in STYLES {
            let style = card_style.as_str();
            let mut prompts = vec![
                (
                    "url generation",
                    build_prompt("u", None, &cfg(12), &styled(card_style)),
                ),
                (
                    "source generation",
                    build_prompt("src", Some(&source_root), &cfg(12), &styled(card_style)),
                ),
                (
                    "review",
                    build_review_prompt("## Q\nA\n", &styled(card_style)),
                ),
            ];
            prompts.push(("fill", fill(&styled(card_style))));
            for (consumer, prompt) in prompts {
                for phrase in [
                    "Do not default every card to one shape",
                    "Choose each card's shape from the shared guide",
                ] {
                    assert!(
                        !prompt.contains(phrase),
                        "{style} {consumer} prompt carries the intro line {phrase:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_generation_prompt_carries_the_self_check_list() {
        for card_style in [GenerateCardStyle::Mixed, GenerateCardStyle::Cloze] {
            assert!(
                build_prompt("u", None, &cfg(12), &styled(card_style))
                    .contains("**Self-check before you hand it over.**"),
                "{} generation prompt lacks the self-check list",
                card_style.as_str()
            );
        }
    }

    #[test]
    fn only_the_exact_default_generation_spec_is_default() {
        let cases = [
            ("the exact default", spec(), true),
            (
                "a different goal",
                GenerationSpec {
                    goal: "learn one topic".to_string(),
                    ..spec()
                },
                false,
            ),
            (
                "an output language",
                GenerationSpec {
                    language: Some("German".to_string()),
                    ..spec()
                },
                false,
            ),
            (
                "an audience",
                GenerationSpec {
                    audience: Some("new programmers".to_string()),
                    ..spec()
                },
                false,
            ),
            (
                "a fixed card style",
                GenerationSpec {
                    card_style: GenerateCardStyle::Plain,
                    ..spec()
                },
                false,
            ),
        ];

        for (name, candidate, expected) in cases {
            assert_eq!(expected, candidate.is_default(), "{name}");
        }
    }

    #[test]
    fn run_config_derives_the_exact_tool_root_and_source_access_policy() {
        let ask = AskConfig {
            allowed_tools: vec!["ParentTool".to_string()],
            cwd: Some(PathBuf::from("/parent")),
            source_access: true,
            ..AskConfig::default()
        };
        let cases = [
            (
                "a URL keeps the parent's network tools but takes the requested root",
                true,
                Some(PathBuf::from("/url-root")),
                vec!["ParentTool".to_string()],
                Some(PathBuf::from("/url-root")),
            ),
            (
                "a local source gets read-only local tools and clears the parent root",
                false,
                None,
                vec!["Read".to_string(), "Glob".to_string(), "Grep".to_string()],
                None,
            ),
        ];

        for (name, url, cwd, expected_tools, expected_cwd) in cases {
            let derived = run_config(&cfg(10), &ask, url, cwd);
            assert_eq!(
                expected_tools, derived.allowed_tools,
                "{name}: allowed tools"
            );
            assert_eq!(expected_cwd, derived.cwd, "{name}: working directory");
            assert!(
                !derived.source_access,
                "{name}: source access is always off"
            );
        }
    }

    #[test]
    fn prompt_substitutes_url_and_card_count() {
        let p = build_prompt("https://example.org/page", None, &cfg(12), &spec());
        assert!(p.contains("https://example.org/page"));
        assert!(p.contains("aiming for at most 12 cards"));
        assert!(p.contains("link: https://example.org/page"));
        assert!(p.contains("## "));
        assert!(p.contains("Cover the material at four levels"));
        assert!(!p.contains("{url}"));
        assert!(!p.contains("{max_cards}"));
        assert!(p.contains("blank: span hidden="));
        assert!(p.contains("hidden=\"dropped\""));
        assert!(p.contains("A cloze card has no answer section"));
        assert!(p.contains("Every card except a cloze card needs at least one answer line"));
        assert!(p.contains("Most cards deserve a `> [!NOTE]` note"));
        assert!(p.contains("No two cards may test the same fact."));
        assert!(p.contains("**Revise before finishing.**"));
        assert!(p.contains("cover exactly what the front asks"));
        assert!(p.contains("one row per pair"));
        assert!(!p.contains("indented answer"));
    }

    #[test]
    fn the_card_shape_guide_teaches_the_current_review_key() {
        assert!(
            part(Part::Shapes).contains("`review:`"),
            "the shapes part omits the `review:` key"
        );
        assert!(
            !part(Part::Guide).contains("`order:`"),
            "the guide teaches an `order:` key the format does not define"
        );
    }

    #[test]
    fn the_mixed_generation_prompt_embeds_the_format_part_and_its_syntax() {
        let prompt = build_prompt("https://example.org/page", None, &cfg(12), &spec());

        assert!(
            prompt.contains(&part(Part::Format)),
            "the mixed prompt must carry the guide part Format"
        );
        for syntax in [
            "| front | back | note |",
            "<!-- reveal: line -->",
            "<!-- input: draw -->",
            "<!-- direction: both -->",
        ] {
            assert!(
                prompt.contains(syntax),
                "the mixed prompt must teach `{syntax}`"
            );
        }
    }

    #[test]
    fn the_review_prompt_for_mixed_style_omits_the_diagram_row_while_generation_keeps_it_only_with_diagrams_on()
     {
        let row_start = "A process that branches";
        assert!(part(Part::Shapes).contains(row_start));
        for diagrams in [true, false] {
            let switched = GenerationSpec { diagrams, ..spec() };
            let generation = build_prompt("https://example.org/page", None, &cfg(12), &switched);
            assert_eq!(
                diagrams,
                generation.contains(row_start),
                "Mixed generation, diagrams {diagrams}: the row is expected only with diagrams on"
            );
            let review = build_review_prompt("## front\nback\n", &switched);
            assert!(
                !review.contains(row_start),
                "Mixed review, diagrams {diagrams}: the review prompt carries the row"
            );
            assert!(
                review.contains("| Ordered steps in one straight line"),
                "Mixed review, diagrams {diagrams}: the review prompt lost the other shape rows"
            );
        }
    }

    #[test]
    fn each_explicit_card_style_keeps_its_override_contract() {
        let withheld = lines_unique_to(
            &[part(Part::Shapes), part(Part::Diagrams)],
            &[part(Part::Format), part(Part::Practice)],
        );
        assert!(
            !withheld.is_empty(),
            "the shapes and diagrams parts have no line of their own"
        );
        let cases = [
            (
                GenerateCardStyle::Plain,
                "Write every card as a plain question and answer",
            ),
            (GenerateCardStyle::Cloze, "Write EVERY card as cloze"),
            (
                GenerateCardStyle::AuthoredChoices,
                "Write EVERY card as authored multiple-choice",
            ),
        ];
        let source_root = src_root();

        for (card_style, contract) in cases {
            let style = card_style.as_str();
            for diagrams in [false, true] {
                for (name, root) in [("a url", None), ("a source root", Some(&source_root))] {
                    let prompt = build_prompt(
                        "notes",
                        root,
                        &cfg(12),
                        &GenerationSpec {
                            card_style,
                            diagrams,
                            ..spec()
                        },
                    );
                    let case = format!("{style}, {name}, diagrams {diagrams}");
                    assert!(
                        prompt.contains(contract),
                        "{case}: must keep its override contract"
                    );
                    assert!(
                        matches!(
                            (prompt.find(PRECEDENCE_LINE), prompt.find(contract)),
                            (Some(line), Some(rule)) if line < rule
                        ),
                        "{case}: the style rule must follow the precedence line"
                    );
                    for (part_name, which) in
                        [("format", Part::Format), ("practice", Part::Practice)]
                    {
                        assert!(
                            prompt.contains(&part(which)),
                            "{case}: lacks the {part_name} part"
                        );
                    }
                    let lines = whole_lines(&prompt);
                    for line in &withheld {
                        assert!(
                            !lines.contains(line.as_str()),
                            "{case}: carries the mixed-shape line {line:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn authored_choice_prompt_allows_its_badged_note_after_the_invocation() {
        let choices = GenerationSpec {
            card_style: GenerateCardStyle::AuthoredChoices,
            ..spec()
        };
        for (source, url) in [("https://example.org/page", true), ("src/lib.rs", false)] {
            let prompt = build_prompt(source, (!url).then(src_root).as_ref(), &cfg(12), &choices);

            assert!(
                !prompt.contains("Nothing belonging to the card may follow a directive"),
                "that absolute rule forbids the note the authored-choice contract requires ({source}): {prompt}"
            );
            assert!(
                prompt.contains("binds the block directly above it"),
                "and the satisfiable order replaces it rather than leaving the question open ({source}): {prompt}"
            );
        }
    }

    #[test]
    fn the_authored_choice_shape_the_prompt_teaches_parses_and_binds() {
        let taught = "## Pick one\n- [x] right\n- [ ] wrong\n- [ ] also wrong\n\
                      <!-- choices: single -->\n> [!NOTE]\n> the mistaken premise\n";
        let parsed = parser::parse("deck.md", taught).expect("the taught shape parses");
        let card = &parsed.cards[0];
        assert_eq!(
            vec!["wrong".to_string(), "also wrong".to_string()],
            card.authored_distractors,
            "the invocation binds the option list it sits under: {card:?}"
        );
        assert_eq!(
            Some("the mistaken premise"),
            card.only_note(),
            "and the note after it is still a note: {card:?}"
        );

        let inverted = "## Pick one\n- [x] right\n- [ ] wrong\n- [ ] also wrong\n\
                        > [!NOTE]\n> the mistaken premise\n<!-- choices: single -->\n";
        assert!(
            parser::parse("deck.md", inverted).is_err(),
            "and the other order does not parse, which is why the order is taught"
        );
    }

    /// The same question the authored-choice contract failed: can a card obey
    /// every rule the prompts state about it at once? A card table cannot
    /// carry a blockquote note in EITHER position, so the note rule has to
    /// name its exception or a generated deck fails to open.
    #[test]
    fn a_card_table_carries_its_note_in_a_column_and_nowhere_else() {
        let column = "## T\n| front | back | note |\n| --- | --- | --- |\n| a | b | n |\n\
                      <!-- cards -->\n";
        let parsed = parser::parse("deck.md", column).expect("the note column parses");
        assert_eq!(
            Some("n"),
            parsed.cards[0].only_note(),
            "a card table's note is its column: {parsed:?}"
        );

        for (position, text) in [
            (
                "below the invocation",
                "## T\n| front | back |\n| --- | --- |\n| a | b |\n<!-- cards -->\n\
                 > [!NOTE]\n> why\n",
            ),
            (
                "between the table and its invocation",
                "## T\n| front | back |\n| --- | --- |\n| a | b |\n> [!NOTE]\n> why\n\
                 <!-- cards -->\n",
            ),
        ] {
            assert!(
                parser::parse("deck.md", text).is_err(),
                "a blockquote note {position} does not parse, which is why the \
                 prompts name the exception"
            );
        }

        for prompt in [
            build_prompt("https://example.org/page", None, &cfg(12), &spec()),
            build_prompt("src/lib.rs", Some(&src_root()), &cfg(12), &spec()),
        ] {
            assert!(
                prompt.contains("A card table's note is that column"),
                "and the prompt teaching card tables states it: {prompt}"
            );
        }
    }

    /// The narrow half of the same rule: a table invocation must keep the
    /// trailing directives the prompts separately REQUIRE, or the repair for
    /// the note trades a broken deck for a deck missing its provenance.
    #[test]
    fn a_card_table_invocation_keeps_the_directives_the_prompts_require() {
        let cited = parser::parse(
            "deck.md",
            "| front | back |\n| --- | --- |\n| a | b |\n<!-- cards -->\n\
             <!-- at: src:src/lib.rs:1-2 -->\n",
        )
        .expect("a table invocation carries the locator the prompt requires");
        assert_eq!(
            "src:src/lib.rs:1-2",
            cited.cards[0].citations[0].locator.to_string(),
            "the citation survives on a row card: {cited:?}"
        );

        for (surface, prompt) in [
            (
                "the local-source generation prompt",
                build_prompt("src/lib.rs", Some(&src_root()), &cfg(12), &spec()),
            ),
            (
                "the URL generation prompt",
                build_prompt("https://example.org/page", None, &cfg(12), &spec()),
            ),
        ] {
            assert!(
                !prompt.contains("takes nothing after it"),
                "{surface} must not forbid the trailing directives it asks for \
                 elsewhere: {prompt}"
            );
        }
    }

    /// The two combinations the earlier probes never put together: a card
    /// table plus the reveal directive the prompts offer it, and a choice card
    /// carrying THREE trailing things at once. Both parse, so the prompts may
    /// keep teaching them.
    #[test]
    fn the_remaining_joint_trailing_shapes_parse_in_prompt_order() {
        let table = parser::parse(
            "deck.md",
            "| front | back |\n| --- | --- |\n| a | b |\n<!-- cards -->\n\
             <!-- reveal: line -->\n",
        )
        .expect("a card-table invocation carries the taught reveal directive");
        assert!(
            table
                .cards
                .iter()
                .all(|card| card.reveal == Some(crate::depth::Reveal::Line)),
            "and the mode reaches every row card: {table:?}"
        );

        let choice = parser::parse(
            "deck.md",
            "## Pick one\n- [x] right\n- [ ] wrong\n- [ ] also wrong\n\
             <!-- choices: single -->\n<!-- at: src:src/lib.rs:1-2 -->\n\
             > [!NOTE]\n> why the alternatives fail\n",
        )
        .expect("a choice invocation carries a locator and then its taught note");
        let card = &choice.cards[0];
        assert_eq!(
            Some("why the alternatives fail"),
            card.only_note(),
            "the note survives two directives above it: {card:?}"
        );
        assert_eq!(
            "src:src/lib.rs:1-2",
            card.citations[0].locator.to_string(),
            "so does the locator: {card:?}"
        );
        assert_eq!(
            vec!["wrong", "also wrong"],
            card.authored_distractors,
            "and the invocation still binds its list: {card:?}"
        );
    }

    #[test]
    fn readme_quickstart_teaches_the_admonition_note_shape() {
        let readme = include_str!("../README.md");
        let explanation = readme
            .split_once("The front is the `## ` line")
            .map(|(_, tail)| tail.lines().take(5).collect::<Vec<_>>().join("\n"))
            .expect("the quickstart explanation remains present");

        assert!(
            explanation.contains("[!NOTE]"),
            "the prose below the badged example must name the badge: {explanation}"
        );
        assert!(
            !explanation.contains("A `> ` line\nis a note"),
            "a bare blockquote is answer quotation now, not a note: {explanation}"
        );
    }

    #[test]
    fn every_prompt_that_teaches_cloze_teaches_an_answerless_card() {
        let styles = [
            GenerateCardStyle::Mixed,
            GenerateCardStyle::Plain,
            GenerateCardStyle::Cloze,
            GenerateCardStyle::AuthoredChoices,
        ];
        crate::testutil::assert_teaches_answerless_cloze(
            "deck-authoring guide",
            &part(Part::Guide),
        );
        for card_style in styles {
            let style_spec = GenerationSpec {
                card_style,
                ..spec()
            };
            for url in [true, false] {
                for diagrams in [false, true] {
                    let switched = GenerationSpec {
                        diagrams,
                        ..style_spec.clone()
                    };
                    let prompt =
                        build_prompt("src", (!url).then(src_root).as_ref(), &cfg(12), &switched);
                    crate::testutil::assert_teaches_answerless_cloze(
                        &format!(
                            "{} generate prompt (url {url}, diagrams {diagrams})",
                            card_style.as_str()
                        ),
                        &prompt,
                    );
                }
            }
            let review = build_review_prompt("## Q\nA\n", &style_spec);
            crate::testutil::assert_teaches_answerless_cloze(
                &format!("{} review prompt", card_style.as_str()),
                &review,
            );
            crate::testutil::assert_teaches_answerless_cloze(
                &format!("{} fill prompt", card_style.as_str()),
                &fill(&style_spec),
            );
        }
    }

    #[test]
    fn mixed_review_preserves_pair_mappings_as_card_tables() {
        let prompt = build_review_prompt("## Draft\nAnswer\n", &spec());

        assert!(prompt.contains("card table"), "prompt: {prompt}");
        assert!(!prompt.contains("as one cloze card"), "prompt: {prompt}");
    }

    #[test]
    fn review_prompt_embeds_the_deck_and_asks_to_dedupe() {
        let p = build_review_prompt("---\nlink: u\n---\n\n## Q\nA\n", &spec());
        assert!(p.contains("## Q"));
        assert!(p.contains("MERGE cards that test the same fact"));
        assert!(p.contains("Output ONLY the improved deck"));
        assert!(p.contains("must ask and tell the same thing"));
        assert!(p.contains("one row per pair"));
        assert!(p.ends_with("---\nlink: u\n---\n\n## Q\nA\n"));
        assert!(p.contains("`---` frontmatter block"));
        assert!(
            p.contains("Keep every `# ` topic heading exactly as written"),
            "the reviewer keeps the headings the sources are bound to"
        );
    }

    #[test]
    fn extra_guidance_is_appended() {
        let mut g = cfg(10);
        g.extra = Some("Focus on the public API.".to_string());
        let p = build_prompt("u", None, &g, &spec());
        assert!(p.contains("Additional instructions:"));
        assert!(p.contains("Focus on the public API."));
    }

    #[test]
    fn prompt_carries_goal_language_and_authored_choice_contract() {
        let spec = GenerationSpec {
            goal: "recognize the constitutional institutions".to_string(),
            language: Some("German".to_string()),
            audience: Some("German high-school students".to_string()),
            card_style: GenerateCardStyle::AuthoredChoices,
            diagrams: false,
        };
        let p = build_prompt("u", None, &cfg(10), &spec);

        assert!(p.contains("recognize the constitutional institutions"));
        assert!(p.contains("German"));
        assert!(p.contains("German high-school students"));
        assert!(p.contains("- [x]"));
        assert!(p.contains("- [ ]"));
        assert!(p.contains("exactly one checked"));
        assert!(p.contains(choice::NOTE_POSITION_INSTRUCTION));
        assert!(!p.contains("no bullet"));
    }

    #[test]
    fn authored_choice_prompt_contract_url_teaches_the_required_trailing_invocation() {
        let spec = GenerationSpec {
            goal: "learn it".to_string(),
            language: None,
            audience: None,
            card_style: GenerateCardStyle::AuthoredChoices,
            diagrams: false,
        };

        let prompt = build_prompt("https://example.org", None, &cfg(10), &spec);

        assert!(
            prompt.contains("<!-- choices: single -->"),
            "the URL template has no deck-wide choices default, so the explicit style must teach the trailing invocation: {prompt}"
        );
    }

    /// Every prompt that teaches the task-list shape must also teach the
    /// invocation that turns it into a card: the URL template carries no
    /// `choices:` default, so a taught shape without it is a checklist.
    #[test]
    fn every_card_style_teaching_task_list_options_also_teaches_the_invocation() {
        for style in STYLES {
            for consumer in [FormatConsumer::Generation, FormatConsumer::ReviewAndFill] {
                let format = card_format(&styled(style), consumer);
                if !format.contains("- [x]") {
                    continue;
                }
                assert!(
                    format.contains("<!-- choices: single -->"),
                    "{style:?} for {consumer:?} teaches task-list options without the \
                     trailing invocation, so its output parses as a literal checklist: {format}"
                );
            }
        }
    }

    #[test]
    fn the_guides_distractor_count_sits_inside_every_second_source() {
        assert!(
            part(Part::Practice).contains("two or three distractors"),
            "the guide's practice part must state its distractor count"
        );

        let spec = GenerationSpec {
            goal: "learn it".to_string(),
            language: None,
            audience: None,
            card_style: GenerateCardStyle::AuthoredChoices,
            diagrams: false,
        };
        for distractors in [2, 3] {
            let options: String = (0..distractors)
                .map(|n| format!("- [ ] Wrong {n}\n"))
                .collect();
            let deck = format!("## Pick one\n- [x] Right\n{options}<!-- choices: single -->\n");
            assert!(
                validate_card_style(&deck, &spec).is_ok(),
                "validate_card_style must accept {distractors} distractors"
            );
        }

        let rule = style_rule(GenerateCardStyle::AuthoredChoices)
            .expect("the authored-choices style has a rule");
        let (before, _) = rule
            .split_once(" GitHub task-list options")
            .expect("the authored-choices rule names its task-list options");
        let range = before.rsplit(' ').next().unwrap_or_default();
        let (low, high) = range
            .split_once('-')
            .and_then(|(low, high)| Some((low.parse::<usize>().ok()?, high.parse::<usize>().ok()?)))
            .expect("the rule's option count reads as low-high");
        let options = low..=high;
        for distractors in [2, 3] {
            assert!(
                options.contains(&(distractors + 1)),
                "the rule's option range {range:?} must contain {distractors} distractors plus the answer"
            );
        }

        let configured = crate::config::AiConfig::default().distractor_count;
        assert!(
            (2..=3).contains(&configured),
            "the default distractor_count {configured} must be two or three"
        );
    }

    #[test]
    fn authored_choice_prompt_contract_url_shape_survives_the_validator() {
        let spec = GenerationSpec {
            goal: "learn it".to_string(),
            language: None,
            audience: None,
            card_style: GenerateCardStyle::AuthoredChoices,
            diagrams: false,
        };
        let following_the_prompt = "---\nlink: https://example.org\n\n---\n## Pick one\n- [x] Right\n- [ ] Wrong A\n- [ ] Wrong B\n<!-- choices: single -->\n";
        let without_the_invocation = "---\nlink: https://example.org\n\n---\n## Pick one\n- [x] Right\n- [ ] Wrong A\n- [ ] Wrong B\n";

        assert!(
            validate_card_style(following_the_prompt, &spec).is_ok(),
            "the explicit URL prompt's prescribed task-list shape must pass its own validator"
        );
        assert!(
            validate_card_style(without_the_invocation, &spec).is_err(),
            "the shape the prompt used to prescribe is exactly what the validator rejects, \
             which is why the instruction has to name the invocation"
        );
    }

    #[test]
    fn authored_choice_prompt_contract_local_source_has_a_deck_default() {
        let spec = GenerationSpec {
            goal: "learn it".to_string(),
            language: None,
            audience: None,
            card_style: GenerateCardStyle::AuthoredChoices,
            diagrams: false,
        };

        let prompt = build_prompt(".", Some(&src_root()), &cfg(10), &spec);

        assert!(prompt.contains("choices: single"));
        assert!(
            validate_card_style(
                "---\nchoices: single\n---\n## Pick one\n- [x] Right\n- [ ] Wrong A\n- [ ] Wrong B\n",
                &spec,
            )
            .is_ok(),
            "the local-source template's deck default keeps the same bare task list valid"
        );
    }

    #[test]
    fn generation_spec_reads_configured_defaults() {
        let config = GenerateDeckConfig {
            language: Some("German".to_string()),
            audience: Some("new voters".to_string()),
            card_style: GenerateCardStyle::AuthoredChoices,
            ..GenerateDeckConfig::default()
        };

        let spec = GenerationSpec::from_config("recognize institutions", &config);

        assert_eq!("recognize institutions", spec.goal);
        assert_eq!(Some("German".to_string()), spec.language);
        assert_eq!(Some("new voters".to_string()), spec.audience);
        assert_eq!(GenerateCardStyle::AuthoredChoices, spec.card_style);
    }

    #[test]
    fn explicit_controls_are_appended_to_a_custom_prompt() {
        let mut config = cfg(5);
        config.prompt = Some("Custom prompt for {source}.".to_string());
        let cases = [
            GenerationSpec {
                goal: "recognize institutions".to_string(),
                ..spec()
            },
            GenerationSpec {
                language: Some("German".to_string()),
                ..spec()
            },
            GenerationSpec {
                audience: Some("new voters".to_string()),
                ..spec()
            },
            GenerationSpec {
                card_style: GenerateCardStyle::AuthoredChoices,
                ..spec()
            },
        ];

        for spec in cases {
            let prompt = build_prompt("notes.md", Some(&src_root()), &config, &spec);
            assert!(prompt.starts_with("Custom prompt for notes.md."));
            assert!(prompt.contains("GENERATION REQUIREMENTS"));
            assert!(prompt.contains(&spec.goal));
            if let Some(language) = spec.language {
                assert!(prompt.contains(&language));
            }
            if let Some(audience) = spec.audience {
                assert!(prompt.contains(&audience));
            }
            if spec.card_style == GenerateCardStyle::AuthoredChoices {
                assert!(prompt.contains("exactly one checked"));
            }
        }
    }

    #[test]
    fn review_prompt_preserves_language_and_authored_choices() {
        let spec = GenerationSpec {
            goal: "learn the topic".to_string(),
            language: Some("de".to_string()),
            audience: Some("beginners".to_string()),
            card_style: GenerateCardStyle::AuthoredChoices,
            diagrams: false,
        };
        let p = build_review_prompt("## Frage\n- [x] Ja\n- [ ] Nein\n", &spec);

        assert!(p.contains("de"));
        assert!(p.contains("beginners"));
        assert!(p.contains("authored multiple-choice"));
        assert!(p.contains("exactly one checked"));
        assert!(p.contains("one pair per"));
        assert!(p.contains(choice::NOTE_POSITION_INSTRUCTION));
        assert!(!p.contains("as one cloze card"));
    }

    #[test]
    fn custom_generation_prompts_still_forbid_position_dependent_choice_notes() {
        let mut config = cfg(5);
        config.prompt = Some("Custom prompt for {source}.".to_string());

        let prompt = build_prompt("notes.md", Some(&src_root()), &config, &spec());

        assert!(prompt.contains(choice::NOTE_POSITION_INSTRUCTION));
    }

    #[test]
    fn review_deck_runs_the_backend_and_returns_the_checked_style() {
        use crate::testutil::{ask_config, exec_lock, fake_reply};

        let _guard = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(
            dir.path(),
            "---\nchoices: single\n---\n## Pick one\n- [ ] Wrong A\n- [x] Correct\n- [ ] Wrong B\n",
        );
        let spec = GenerationSpec {
            goal: "learn it".to_string(),
            language: None,
            audience: None,
            card_style: GenerateCardStyle::AuthoredChoices,
            diagrams: false,
        };

        let reviewed = review_deck(
            "## Draft\n- [ ] A\n- [x] B\n- [ ] C\n",
            None,
            &cfg(10),
            &ask_config(&cli),
            &spec,
        )
        .unwrap();

        assert!(reviewed.text.contains("- [x] Correct"));
    }

    #[test]
    fn authored_choice_generation_rejects_plain_cards() {
        use crate::testutil::{ask_config, exec_lock, fake_reply};

        let _guard = exec_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_reply(dir.path(), "## Plain question\nPlain answer\n");
        let spec = GenerationSpec {
            goal: "learn the topic".to_string(),
            language: None,
            audience: None,
            card_style: GenerateCardStyle::AuthoredChoices,
            diagrams: false,
        };

        let err = generate_deck(
            "https://example.org",
            None,
            &cfg(10),
            &ask_config(&cli),
            &spec,
        )
        .unwrap_err();

        assert!(format!("{err:#}").contains("authored multiple-choice"));
    }

    #[test]
    fn malformed_output_is_tolerated_only_when_no_explicit_card_style_was_requested() {
        let malformed = "---\nformat-version: 1\nbroken: [\n---\n## Question\nAnswer\n";

        for card_style in [
            GenerateCardStyle::Plain,
            GenerateCardStyle::Cloze,
            GenerateCardStyle::AuthoredChoices,
        ] {
            let spec = GenerationSpec {
                goal: "learn it".to_string(),
                language: None,
                audience: None,
                card_style,
                diagrams: false,
            };
            let error = validate_card_style(malformed, &spec).unwrap_err();
            assert!(
                format!("{error:#}").contains("cannot verify generated card style"),
                "{card_style:?}: {error:#}"
            );
        }

        validate_card_style(malformed, &spec()).unwrap();
    }

    #[test]
    fn authored_choices_require_three_to_five_options() {
        let spec = GenerationSpec {
            goal: "learn it".to_string(),
            language: None,
            audience: None,
            card_style: GenerateCardStyle::AuthoredChoices,
            diagrams: false,
        };

        let too_few = validate_card_style("## Pick one\n- [x] Right\n- [ ] Wrong\n", &spec);
        let too_many = validate_card_style(
            "## Pick one\n- [x] Right\n- [ ] A\n- [ ] B\n- [ ] C\n- [ ] D\n- [ ] E\n",
            &spec,
        );

        assert!(too_few.is_err());
        assert!(too_many.is_err());
    }

    #[test]
    fn a_generated_choice_note_cannot_name_a_position_that_shuffle_changes() {
        let deck = "---\nchoices: single\n---\n## Pick one\n- [x] Correct claim\n- [ ] First misconception\n- [ ] Second misconception\n> [!NOTE]\n> Option 2 reverses the relation.\n";

        for card_style in [GenerateCardStyle::Mixed, GenerateCardStyle::AuthoredChoices] {
            let spec = GenerationSpec {
                goal: "learn it".to_string(),
                language: None,
                audience: None,
                card_style,
                diagrams: false,
            };
            let error = validate_card_style(deck, &spec).unwrap_err();

            assert!(
                format!("{error:#}").contains("shuffled"),
                "{card_style:?}: {error:#}"
            );
        }
    }

    #[test]
    fn each_explicit_card_style_accepts_its_canonical_shape() {
        let cases = [
            (GenerateCardStyle::Plain, "## Question\nAnswer\n"),
            (
                GenerateCardStyle::Cloze,
                "## Complete it\nThe value is dropped.\n<!-- blank: span hidden=\"dropped\" -->\n",
            ),
            (
                GenerateCardStyle::AuthoredChoices,
                "---\nchoices: single\n---\n## Pick one\n- [ ] Wrong A\n- [x] Correct\n- [ ] Wrong B\n",
            ),
        ];

        for (card_style, deck) in cases {
            let spec = GenerationSpec {
                goal: "learn it".to_string(),
                language: None,
                audience: None,
                card_style,
                diagrams: false,
            };
            validate_card_style(deck, &spec).unwrap();
        }
    }

    #[test]
    fn span_blank_cards_satisfy_cloze_style_and_not_plain_style() {
        let deck =
            "## Complete it\nThe value is dropped.\n<!-- blank: span hidden=\"dropped\" -->\n";
        let spec = |card_style| GenerationSpec {
            goal: "learn it".to_string(),
            language: None,
            audience: None,
            card_style,
            diagrams: false,
        };
        validate_card_style(deck, &spec(GenerateCardStyle::Cloze))
            .expect("a span blank IS the requested cloze shape");
        assert!(
            validate_card_style(deck, &spec(GenerateCardStyle::Plain)).is_err(),
            "a span blank is not a plain card"
        );
    }

    #[test]
    fn cloze_style_rejects_a_plain_card() {
        let spec = GenerationSpec {
            goal: "learn it".to_string(),
            language: None,
            audience: None,
            card_style: GenerateCardStyle::Cloze,
            diagrams: false,
        };

        let error = validate_card_style("## Question\nAnswer\n", &spec).unwrap_err();

        assert!(format!("{error:#}").contains("requested cloze style"));
    }

    #[test]
    fn plain_style_rejects_cloze_and_authored_choices() {
        let spec = GenerationSpec {
            goal: "learn it".to_string(),
            language: None,
            audience: None,
            card_style: GenerateCardStyle::Plain,
            diagrams: false,
        };

        let cloze = validate_card_style(
            "## Complete it\nIt is done.\n<!-- blank: span hidden=\"done\" -->\n",
            &spec,
        );
        let choices = validate_card_style(
            "---\nchoices: single\n---\n## Pick one\n- [ ] Wrong A\n- [x] Correct\n- [ ] Wrong B\n",
            &spec,
        );

        assert!(cloze.is_err());
        assert!(choices.is_err());
    }

    #[test]
    fn full_prompt_override_replaces_template_but_keeps_note_safety() {
        let mut g = cfg(5);
        g.prompt = Some("Make {max_cards} cards from {url}.".to_string());
        let p = build_prompt("U", None, &g, &spec());
        assert!(p.starts_with("Make 5 cards from U."));
        assert!(p.contains(choice::NOTE_POSITION_INSTRUCTION));
        assert!(!p.contains("**Make the learner think.**"));
        assert!(!p.contains(PRECEDENCE_LINE));
    }

    #[test]
    fn source_prompt_explores_locally_and_ties_to_source() {
        let p = build_prompt("src/scheduler.rs", Some(&src_root()), &cfg(8), &spec());
        assert!(p.contains("src/scheduler.rs"));
        assert!(p.contains("Read, Glob and Grep"));
        assert!(p.contains("aiming for at most 8 cards"));
        assert!(!p.contains("WebFetch"));
        assert!(!p.contains("{source}"));
        assert!(p.contains("`src:<file>:start-end`"));
        assert!(p.contains("`<!-- at: src:src/string.rs:120-128 -->`"));
        assert!(p.contains("`src:\"my notes.md\":3-4`"));
        assert!(p.contains("never guess"));
        assert!(p.contains("one row per pair"));
        assert!(!p.contains("indented answer"));
    }

    #[test]
    fn declare_root_adds_the_generated_root_once_and_keeps_every_other_line() {
        let declaring = GeneratedRoot {
            declare: Some("/in".into()),
            ..src_root()
        };
        for (label, root, text, expected) in [
            (
                "a reused root declares nothing",
                src_root(),
                "---\ntitle: x\n---\n## Q\nA\n",
                "---\ntitle: x\n---\n## Q\nA\n",
            ),
            (
                "a deck with frontmatter gains the cite entry",
                declaring.clone(),
                "---\ntitle: x\n---\n## Q\nA\n",
                "---\ntitle: x\ncite:\n  - src: \"/in\"\n---\n## Q\nA\n",
            ),
            (
                "a deck without frontmatter gains one",
                declaring.clone(),
                "## Q\nA\n",
                "---\ncite:\n  - src: \"/in\"\n---\n\n## Q\nA\n",
            ),
            (
                "a deck already declaring the root is unchanged",
                declaring.clone(),
                "---\ncite:\n  - src: /in\n---\n## Q\nA\n",
                "---\ncite:\n  - src: /in\n---\n## Q\nA\n",
            ),
        ] {
            let out = declare_root(text, &root).unwrap();
            assert_eq!(expected, out, "row `{label}`");
            if root.declare.is_some() {
                assert_eq!(
                    vec![("src".to_string(), "/in".to_string())],
                    parser::parse("d.md", &out).unwrap().frontmatter.cite,
                    "row `{label}` parses back"
                );
            }
        }
    }

    #[test]
    fn url_prompt_does_not_ask_for_line_citations() {
        let p = build_prompt("https://example.org/page", None, &cfg(8), &spec());
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
    fn clean_preserves_the_frontmatter_fence_spellings_the_parser_accepts() {
        let raw = "Here is your deck:\n\n--- \nlink: u\n---\t\n## Q\nA\n";
        assert_eq!("--- \nlink: u\n---\t\n## Q\nA", clean_output(raw));
    }

    #[test]
    fn clean_strips_commentary_and_fence_together() {
        let raw = "Here is your deck:\n```text\n---\nlink: u\n---\n## Q\nA\n```";
        assert_eq!("---\nlink: u\n---\n## Q\nA", clean_output(raw));
    }

    #[test]
    fn clean_keeps_the_closing_fence_of_a_code_block_ending_the_deck() {
        for (step, raw) in [
            ("bare reply", "## Q\nA\n```rust\nlet x = 1;\n```\n"),
            (
                "wrapped reply",
                "```markdown\n## Q\nA\n```rust\nlet x = 1;\n```\n```\n",
            ),
            (
                "four-backtick wrapper",
                "````text\n## Q\nA\n```rust\nlet x = 1;\n```\n````",
            ),
        ] {
            assert_eq!(
                "## Q\nA\n```rust\nlet x = 1;\n```",
                clean_output(raw),
                "{step}: {raw:?}"
            );
        }
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

    fn generation_workspace() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let decks = base.join("ws").join(crate::workspace::DECKS);
        std::fs::create_dir_all(&decks).unwrap();
        std::fs::write(base.join("ws").join(crate::workspace::MANIFEST), "").unwrap();
        let source = base.join("project");
        std::fs::create_dir_all(&source).unwrap();
        (temp, decks, source)
    }

    fn generate_into(
        decks: &Path,
        source: &Path,
        reply: &str,
    ) -> (
        Result<(crate::library::Placed, crate::sources::commands::AddReport)>,
        String,
    ) {
        use crate::testutil::{ask_config, fake_replies};

        let calls = tempfile::tempdir().unwrap();
        let cli = fake_replies(calls.path(), &[reply]);
        let source = source.display().to_string();
        let root = GeneratedRoot {
            key: "src".into(),
            path_prefix: String::new(),
            declare: Some(source.clone()),
        };
        let placed = generate_deck(&source, Some(&root), &cfg(10), &ask_config(&cli), &spec())
            .and_then(|reply| {
                let text = declare_root(&reply.text, &root)?;
                let (deck, root) = generated_sources(&source, &root, &reply, &text)?;
                crate::library::place_sourced_deck(
                    decks,
                    "gen.md",
                    &deck.text,
                    &deck.topic_sources,
                    root,
                    &crate::config::ExamConfig::default(),
                )
            });
        let prompt = std::fs::read_to_string(calls.path().join("prompt-1")).unwrap();
        (placed, prompt)
    }

    type Rows = Vec<(String, Vec<String>)>;

    fn bound(decks: &Path, placed: &Path) -> (Vec<Option<String>>, Rows) {
        let deck = crate::deck::Deck::load(placed).unwrap();
        let workspace = decks.parent().unwrap();
        let sources = crate::sources::load_verified(workspace, deck.deck_token.as_deref().unwrap())
            .unwrap()
            .expect("a sources manifest");
        let rows = sources
            .manifest
            .sources
            .values()
            .map(|row| (row.path.clone(), row.bound_to.clone()))
            .collect();
        (deck.topic_ids(), rows)
    }

    #[test]
    fn a_generated_deck_binds_each_topics_returned_files_and_a_refused_file_writes_nothing() {
        let _guard = crate::testutil::exec_lock();
        let deck = "---\nchoices: single\n---\n# Alpha\n\n## What is a?\nThe first letter\n<!-- at: src:a.md:1 -->\n\n# Beta\n\n## What is b?\nThe second letter\n";
        let reply = |beta: &str| {
            format!(
                "{deck}{SOURCES_MARKER}\n[{{\"topic\": \"Alpha\", \"sources\": [{{\"path\": \"a.md\", \"lines\": \"1-2\"}}]}}, {{\"topic\": \"Beta\", \"sources\": [{{\"path\": \"{beta}\"}}]}}]\n"
            )
        };

        let (_temp, decks, source) = generation_workspace();
        std::fs::write(source.join("a.md"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(source.join("b.md"), "bee\n").unwrap();
        let (placed, prompt) = generate_into(&decks, &source, &reply("b.md"));
        assert!(
            prompt.contains(SOURCES_MARKER)
                && prompt.contains(&format!("longer than {RANGED_FILE_LINES} lines")),
            "prompt: a folder generation asks for the sources list and its ranges rule"
        );
        assert!(
            !prompt.contains("may name only"),
            "prompt: a folder generation carries no one-file restriction"
        );
        let (placed, report) = placed.expect("binding: the returned files bind");
        assert_eq!(2, report.added.len(), "binding: {:?}", report.added);
        let (ids, rows) = bound(&decks, &placed.path);
        let (alpha, beta) = (ids[1].clone().unwrap(), ids[2].clone().unwrap());
        assert_eq!(
            vec![
                ("src:a.md".to_string(), vec![format!("1-2|{alpha}")]),
                ("src:b.md".to_string(), vec![beta]),
            ],
            {
                let mut rows = rows;
                rows.sort();
                rows
            },
            "binding: each topic binds the files the model returned for it"
        );
        let deck_id = ids[0].clone().unwrap();
        let roots = crate::sources::Roots::load(
            &crate::workspace::WorkspaceFiles::new(decks.parent().unwrap()).sources_for(&deck_id),
        )
        .unwrap()
        .expect("binding: a roots file");
        assert_eq!(
            Some(&source),
            roots.roots.get("src"),
            "binding: the root is saved under the `cite:` key"
        );

        let (_temp, decks, source) = generation_workspace();
        std::fs::write(source.join("a.md"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(source.join("bad.bin"), [0xff, 0xfe, 0x00]).unwrap();
        let (placed, _) = generate_into(&decks, &source, &reply("bad.bin"));
        let message = format!(
            "{:#}",
            placed.expect_err("refusal: a non-UTF-8 file refuses")
        );
        assert!(
            message.contains("bad.bin") && message.contains("Beta"),
            "refusal: the error names the file and the topic heading: {message}"
        );
        assert_eq!(
            0,
            std::fs::read_dir(&decks).unwrap().count(),
            "refusal: no deck file is left"
        );
        let workspace = decks.parent().unwrap();
        for folder in ["sources", "assets"] {
            assert!(
                !workspace.join(folder).exists(),
                "refusal: no `{folder}/` folder is left"
            );
        }
    }

    #[test]
    fn model_shaped_source_lists_bind_like_their_plain_spelling() {
        let deck = "# Alpha\n\n## What is a?\nThe first letter\n\n# Beta\n\n## What is b?\nThe second letter\n";
        let plain = "[{\"topic\": \"Alpha\", \"sources\": [{\"path\": \"a.md\", \"lines\": \"1-2,3\"}]}, {\"topic\": \"Beta\", \"sources\": [{\"path\": \"a.md\"}]}]";
        let cases = [
            ("spaces in ranges", plain.replace("1-2,3", "1 - 2, 3")),
            (
                "a second spelling",
                plain.replace("\"path\": \"a.md\"}", "\"path\": \"sub/../a.md\"}"),
            ),
            ("an outer fence", format!("{plain}\n```")),
            ("a json fence", format!("```json\n{plain}\n```")),
        ];
        let bind = |sources: &str| {
            let (_temp, decks, source) = generation_workspace();
            std::fs::write(source.join("a.md"), "one\ntwo\nthree\n").unwrap();
            std::fs::create_dir(source.join("sub")).unwrap();
            let generated = parse_generated(deck, sources, &source)?;
            let (placed, _) = crate::library::place_sourced_deck(
                &decks,
                "gen.md",
                &generated.text,
                &generated.topic_sources,
                ("src".to_string(), source.clone()),
                &crate::config::ExamConfig::default(),
            )?;
            let (ids, rows) = bound(&decks, &placed.path);
            let names: Vec<String> = ids.into_iter().flatten().collect();
            anyhow::Ok(
                rows.into_iter()
                    .map(|(path, entries)| {
                        let entries = entries
                            .into_iter()
                            .map(|entry| {
                                names.iter().enumerate().fold(entry, |entry, (at, id)| {
                                    entry.replace(id.as_str(), &format!("topic{at}"))
                                })
                            })
                            .collect();
                        (path, entries)
                    })
                    .collect::<Rows>(),
            )
        };
        let expected = bind(plain).unwrap();
        assert_eq!(
            vec![(
                "src:a.md".to_string(),
                vec!["1-2,3|topic1".to_string(), "topic2".to_string()]
            )],
            expected,
            "plain: one row binds both topics"
        );
        for (case, sources) in cases {
            assert_eq!(
                expected,
                bind(&sources).unwrap_or_else(|error| panic!("{case}: {error:#}")),
                "{case}"
            );
        }
    }

    #[test]
    fn a_space_inside_a_range_number_is_kept_for_the_range_check_to_refuse() {
        let listed = parse_source_list(
            "[{\"path\": \"a.md\", \"lines\": \"1 2\"}]",
            Path::new("/p"),
        )
        .unwrap();
        assert_eq!(
            None,
            listed[0].1.as_deref().filter(|lines| *lines == "12"),
            "`1 2` became {listed:?}"
        );
    }

    #[test]
    fn a_heading_that_only_normalization_reveals_shifts_no_binding() {
        let deck = "# Alpha\n\n## q\na\n\n\u{feff}# Hidden\n\n## q2\na2\n\n# Beta\n\n## q3\na3\n";
        let sources = "[{\"topic\": \"Alpha\", \"sources\": [{\"path\": \"a.md\"}]}, {\"topic\": \"Hidden\", \"sources\": [{\"path\": \"h.md\"}]}, {\"topic\": \"Beta\", \"sources\": [{\"path\": \"b.md\"}]}]";

        let generated = parse_generated(deck, sources, Path::new("/project")).unwrap();

        let placed = parser::parse("generated.md", &parser::normalize(deck)).unwrap();
        let beta = placed
            .topics
            .iter()
            .position(|topic| topic.heading.as_deref() == Some("Beta"))
            .unwrap();
        assert_eq!(
            Some(beta),
            generated
                .topic_sources
                .iter()
                .find(|(_, files)| files[0].0.ends_with("b.md"))
                .map(|(index, _)| *index),
            "the index is the one the placed (normalized) deck has"
        );
    }

    #[test]
    fn a_source_list_must_name_every_card_bearing_topic_once_with_files() {
        let deck = "# Alpha\n\n## a?\nA\n\n# Beta\n\n## b?\nB\n\n# Empty\n";
        let alpha = "{\"topic\": \"Alpha\", \"sources\": [{\"path\": \"a.md\"}]}";
        let beta = "{\"topic\": \"Beta\", \"sources\": [{\"path\": \"b.md\"}]}";
        let beta_empty = "{\"topic\": \"Beta\", \"sources\": []}";
        let cases = [
            (
                "complete",
                deck.to_string(),
                format!("[{alpha}, {beta}]"),
                None,
            ),
            (
                "an omitted topic",
                deck.to_string(),
                format!("[{alpha}]"),
                Some("no sources for topic `Beta`"),
            ),
            (
                "an empty list",
                deck.to_string(),
                format!("[{alpha}, {beta_empty}]"),
                Some("no sources for topic `Beta`"),
            ),
            (
                "a topic listed twice",
                deck.to_string(),
                format!("[{alpha}, {beta}, {beta}]"),
                Some("topic `Beta` more than once"),
            ),
            (
                "cards before the first heading",
                format!("## lead?\nL\n\n{deck}"),
                format!("[{alpha}, {beta}]"),
                Some("before its first `#` topic heading"),
            ),
            (
                "no headings at all",
                "## a?\nA\n".to_string(),
                "[]".to_string(),
                Some("before its first `#` topic heading"),
            ),
        ];
        for (case, text, sources, refusal) in cases {
            let result = parse_generated(&text, &sources, Path::new("/project"));
            match refusal {
                None => assert!(result.is_ok(), "{case}: {:#}", result.unwrap_err()),
                Some(expected) => {
                    let message = format!("{:#}", result.expect_err(case));
                    assert!(message.contains(expected), "{case}: {message}");
                }
            }
        }
    }

    const SINGLE_FILE_DECK: &str =
        "---\nchoices: single\n---\n# One\n\n## Q1\nA1\n\n# Two\n\n## Q2\nA2\n";

    fn single_file_reply(one: &str, two: &str) -> String {
        format!(
            "{SINGLE_FILE_DECK}{SOURCES_MARKER}\n[{{\"topic\": \"One\", \"sources\": [{one}]}}, {{\"topic\": \"Two\", \"sources\": [{two}]}}]\n"
        )
    }

    fn listed(path: &str, lines: Option<&str>) -> String {
        let path = serde_json::to_string(path).unwrap();
        match lines {
            Some(lines) => format!("{{\"path\": {path}, \"lines\": \"{lines}\"}}"),
            None => format!("{{\"path\": {path}}}"),
        }
    }

    fn single_file_folder() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let (temp, decks, folder) = generation_workspace();
        std::fs::write(folder.join("notes.md"), "line one\nline two\n").unwrap();
        std::fs::write(folder.join("other.md"), "another file\n").unwrap();
        (temp, decks, folder)
    }

    #[test]
    fn single_file_generation_binds_each_topic_to_what_the_reply_names() {
        let _guard = crate::testutil::exec_lock();
        fn ranged(path: String) -> (String, String) {
            (listed(&path, Some("1")), listed(&path, Some("2")))
        }
        type Row = (
            &'static str,
            Box<dyn Fn(&Path) -> (String, String)>,
            Vec<&'static str>,
        );
        let mut rows: Vec<Row> = vec![
            (
                "per-section ranges",
                Box::new(|_| ranged("notes.md".into())),
                vec!["1|One", "2|Two"],
            ),
            (
                "lines omitted",
                Box::new(|_| (listed("notes.md", None), listed("notes.md", None))),
                vec!["One", "Two"],
            ),
            (
                "a leading dot",
                Box::new(|_| ranged("./notes.md".into())),
                vec!["1|One", "2|Two"],
            ),
            (
                "the absolute path",
                Box::new(|folder| ranged(folder.join("notes.md").display().to_string())),
                vec!["1|One", "2|Two"],
            ),
            (
                "up and back into the folder",
                Box::new(|folder| {
                    let name = folder.file_name().unwrap().to_string_lossy();
                    ranged(format!("../{name}/notes.md"))
                }),
                vec!["1|One", "2|Two"],
            ),
        ];
        #[cfg(unix)]
        rows.push((
            "a symlink to the source file",
            Box::new(|folder| {
                std::os::unix::fs::symlink(folder.join("notes.md"), folder.join("link.md"))
                    .unwrap();
                ranged("link.md".into())
            }),
            vec!["1|One", "2|Two"],
        ));
        for (row, sources, expected) in rows {
            let (_temp, decks, folder) = single_file_folder();
            let (one, two) = sources(&folder);
            let (placed, prompt) = generate_into(
                &decks,
                &folder.join("notes.md"),
                &single_file_reply(&one, &two),
            );
            assert!(
                prompt.contains(SOURCES_MARKER)
                    && prompt.contains("every `path` may name only `notes.md`"),
                "{row}: the prompt asks for the list and names the one file"
            );
            let (placed, _) = placed.unwrap_or_else(|error| panic!("{row}: {error:#}"));
            let (ids, bound_rows) = bound(&decks, &placed.path);
            let topics = [
                (ids[1].clone().unwrap(), "One"),
                (ids[2].clone().unwrap(), "Two"),
            ];
            let named: Rows = bound_rows
                .into_iter()
                .map(|(path, entries)| {
                    let entries = entries
                        .into_iter()
                        .map(|entry| {
                            topics.iter().fold(entry, |entry, (id, topic)| {
                                entry.replace(id.as_str(), topic)
                            })
                        })
                        .collect();
                    (path, entries)
                })
                .collect();
            assert_eq!(
                vec![(
                    "src:notes.md".to_string(),
                    expected.iter().map(|entry| entry.to_string()).collect()
                )],
                named,
                "{row}: only the source file binds, with the named lines"
            );
        }
    }

    #[test]
    fn single_file_generation_refuses_another_file_or_a_missing_list_and_keeps_nothing() {
        let _guard = crate::testutil::exec_lock();
        let one = listed("notes.md", None);
        type Row<'a> = (&'a str, Box<dyn Fn(&Path) -> String + 'a>, &'a str);
        let mut rows: Vec<Row> = vec![
            (
                "a sibling file that exists",
                Box::new(|_| single_file_reply(&one, &listed("other.md", None))),
                "other.md",
            ),
            (
                "a path that does not exist",
                Box::new(|_| single_file_reply(&one, &listed("missing.md", None))),
                "missing.md",
            ),
            (
                "no sources list",
                Box::new(|_| SINGLE_FILE_DECK.to_string()),
                SOURCES_MARKER,
            ),
        ];
        #[cfg(unix)]
        rows.push((
            "a symlink to a sibling file",
            Box::new(|folder| {
                std::os::unix::fs::symlink(folder.join("other.md"), folder.join("link.md"))
                    .unwrap();
                single_file_reply(&one, &listed("link.md", None))
            }),
            "link.md",
        ));
        for (row, reply, named) in rows {
            let (_temp, decks, folder) = single_file_folder();
            let reply = reply(&folder);
            let (placed, _) = generate_into(&decks, &folder.join("notes.md"), &reply);
            let message = format!(
                "{:#}",
                placed.expect_err(&format!("{row}: the reply is refused"))
            );
            assert!(
                message.contains(named),
                "{row}: the error names `{named}`: {message}"
            );
            assert_eq!(
                0,
                std::fs::read_dir(&decks).unwrap().count(),
                "{row}: no deck file is left"
            );
            let workspace = decks.parent().unwrap();
            for kept in ["sources", "assets"] {
                assert!(
                    !workspace.join(kept).exists(),
                    "{row}: no `{kept}/` folder is left"
                );
            }
        }
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
