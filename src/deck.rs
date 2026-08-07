use std::{
    collections::HashSet,
    ops::Range,
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::{
    answer::Input,
    card::{Card, Direction},
    config::Strictness,
    depth::Reveal,
    parser::{self, ParseError},
    session::{self, Order},
    store::Store,
};

#[derive(Debug, Default, Clone)]
pub struct DeckSettings {
    pub reveal: Option<Reveal>,
    pub input: Option<Input>,
    pub order: Option<Order>,
    pub direction: Option<Direction>,
    pub sampling: Option<bool>,
    pub exam_strictness: Option<Strictness>,
}

impl DeckSettings {
    pub fn from_directives(directives: &[(String, String)]) -> Self {
        let mut settings = Self::default();
        for (key, value) in directives {
            match key.as_str() {
                "reveal" => settings.reveal = Reveal::parse(value),
                "input" => settings.input = Input::parse(value),
                "order" => settings.order = Order::parse(value),
                "direction" => settings.direction = Direction::parse(value),
                "sampling" => settings.sampling = parser::parse_sampling(value),
                "strictness" => settings.exam_strictness = Strictness::parse(value),
                _ => {}
            }
        }
        settings
    }

    pub fn from_frontmatter(frontmatter: &parser::Frontmatter) -> Self {
        Self {
            reveal: frontmatter.reveal,
            input: frontmatter.input,
            order: frontmatter.order,
            direction: frontmatter.direction,
            sampling: frontmatter.sampling,
            // Learner setting: a deck never ships grading rigor.
            exam_strictness: None,
        }
    }

    fn fill_from(&mut self, defaults: &DeckSettings) {
        self.reveal = self.reveal.or(defaults.reveal);
        self.input = self.input.or(defaults.input);
        self.order = self.order.or(defaults.order);
        self.direction = self.direction.or(defaults.direction);
        self.sampling = self.sampling.or(defaults.sampling);
        self.exam_strictness = self.exam_strictness.or(defaults.exam_strictness);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeckState {
    NotStarted,
    Started,
    ExamDue,
    Finished,
}

#[derive(Debug)]
pub struct Deck {
    pub path: PathBuf,
    pub subject: String,
    pub deck_token: Option<String>,
    pub cards: Vec<Card>,
    pub links: Vec<String>,
    pub requires: Vec<String>,
    pub sources: Vec<String>,
    pub settings: DeckSettings,
    pub title: Option<String>,
    pub preamble: Option<String>,
    pub trace: Option<String>,
}

#[derive(Debug, Error)]
pub enum DeckError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseError,
    },
    #[error("{path}: file name is not valid UTF-8")]
    InvalidFileName { path: PathBuf },
    #[error("{path}: the card at line {line} is a table row; a note cannot attach to it")]
    TableRowNote { path: PathBuf, line: usize },
}

impl Deck {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, DeckError> {
        Self::load_with_defaults(path, &DeckSettings::default())
    }

    pub fn load_with_defaults(
        path: impl AsRef<Path>,
        defaults: &DeckSettings,
    ) -> Result<Self, DeckError> {
        let path = path.as_ref().to_path_buf();
        let subject = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| DeckError::InvalidFileName { path: path.clone() })?
            .to_string();
        let text = std::fs::read_to_string(&path).map_err(|source| DeckError::Io {
            path: path.clone(),
            source,
        })?;
        let parsed = parser::parse(&subject, &text).map_err(|source| DeckError::Parse {
            path: path.clone(),
            source,
        })?;
        let links = parsed.frontmatter.link.clone();
        let requires = parsed.frontmatter.requires.clone();
        let sources = parsed.frontmatter.source.clone();
        let title = parsed.title.clone();
        let preamble = parsed.preamble.clone();
        let trace = parsed.frontmatter.trace.clone();
        let deck_token = parsed.deck_token.clone();
        let mut settings = DeckSettings::from_frontmatter(&parsed.frontmatter);
        let mut cards = parsed.cards;
        settings.fill_from(defaults);
        for card in &mut cards {
            card.reveal = card.reveal.or(settings.reveal);
            card.input = card.input.or(settings.input);
        }
        // No filesystem check here: a missing image must not stop the deck from loading.
        let base_dir = image_base_dir(&path);
        for card in &mut cards {
            for image in card.images.iter_mut().chain(card.images_back.iter_mut()) {
                image.src = resolve_image(&base_dir, std::mem::take(&mut image.src));
            }
        }
        let mut expanded = Vec::with_capacity(cards.len());
        for card in cards {
            let mut card = card;
            card.sampling = card.sampling.or(settings.sampling);
            let direction = card.direction.or(settings.direction).unwrap_or_default();
            // Keying on the hole (not direction) stops a deck-wide "both" from reversing cloze
            // cards.
            if card.hole.is_some() || direction == Direction::Forward {
                expanded.push(card);
            } else {
                let reversed = card.reversed();
                match direction {
                    Direction::Reverse => expanded.push(reversed),
                    Direction::Both => {
                        expanded.push(card);
                        expanded.push(reversed);
                    }
                    Direction::Forward => unreachable!("handled above"),
                }
            }
        }
        let cards = expanded;
        Ok(Self {
            path,
            subject,
            deck_token,
            cards,
            links,
            requires,
            sources,
            settings,
            title,
            preamble,
            trace,
        })
    }

    pub fn is_trace(&self) -> bool {
        self.trace.is_some()
    }

    pub fn has_exam(&self) -> bool {
        self.is_trace() || !self.sources.is_empty() || !self.workspace_sources().is_empty()
    }

    pub fn display_name(&self) -> String {
        self.title
            .clone()
            .or_else(|| self.trace.clone())
            .unwrap_or_else(|| {
                self.subject
                    .strip_suffix(".md")
                    .unwrap_or(&self.subject)
                    .to_string()
            })
    }

    pub fn state(&self, store: &Store) -> DeckState {
        let total = self.cards.len();
        if total == 0 {
            return DeckState::NotStarted;
        }
        if store.deck_mastered(self.deck_token.as_deref().unwrap_or_default()) {
            return DeckState::Finished;
        }
        let gated = self.cards.iter().all(|c| session::has_graduated(c, store));
        if gated {
            if self.has_exam() {
                DeckState::ExamDue
            } else {
                DeckState::Finished
            }
        } else if self
            .cards
            .iter()
            .all(|c| c.id().and_then(|id| store.progress(&id)).is_none())
        {
            DeckState::NotStarted
        } else {
            DeckState::Started
        }
    }

    pub fn reference_links(&self) -> Vec<String> {
        let mut out = self.links.clone();
        for url in self.source_urls() {
            if !out.contains(&url) {
                out.push(url);
            }
        }
        out
    }

    /// The workspace manifest's `source`: the material the workspace is about,
    /// layered under every member's own sources as supporting context.
    pub fn workspace_sources(&self) -> Vec<String> {
        crate::workspace::manifest_source(&crate::workspace::content_root(&self.path))
    }

    pub fn source_layers(&self) -> SourceLayers {
        let own = self.sources.clone();
        let workspace = self
            .workspace_sources()
            .into_iter()
            .filter(|source| !own.contains(source))
            .collect();
        SourceLayers { own, workspace }
    }

    pub fn source_urls(&self) -> Vec<String> {
        let layers = self.source_layers();
        let mut out = Vec::new();
        for source in layers.own.iter().chain(&layers.workspace) {
            if is_url(source) && !out.contains(source) {
                out.push(source.clone());
            }
        }
        out
    }

    /// The single mechanical base: the deck's first local-path source, falling
    /// back to the workspace source when the deck declares none (ADR 0026).
    pub fn base_root(&self) -> Option<PathBuf> {
        let layers = self.source_layers();
        let local = layers
            .own
            .iter()
            .chain(&layers.workspace)
            .find(|source| !is_url(source))?;
        let content_root = crate::workspace::content_root(&self.path);
        Some(resolve_base_root(local, &content_root))
    }

    pub fn is_frozen(&self) -> bool {
        self.cards.iter().any(|card| {
            card.citations
                .iter()
                .any(|citation| citation.asset.is_some())
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceLayers {
    pub own: Vec<String>,
    pub workspace: Vec<String>,
}

impl SourceLayers {
    pub fn is_empty(&self) -> bool {
        self.own.is_empty() && self.workspace.is_empty()
    }

    /// The mechanical base layer (ADR 0026): the deck's own local-path
    /// sources; the workspace layer participates only when the deck declares
    /// no local one.
    pub fn base_locals(&self) -> Vec<&String> {
        let own: Vec<&String> = self.own.iter().filter(|source| !is_url(source)).collect();
        if own.is_empty() {
            self.workspace
                .iter()
                .filter(|source| !is_url(source))
                .collect()
        } else {
            own
        }
    }
}

fn resolve_base_root(source: &str, content_root: &Path) -> PathBuf {
    let source = Path::new(source.trim());
    let path = if source.is_absolute() {
        source.to_path_buf()
    } else {
        content_root.join(source)
    };
    let path = path.canonicalize().unwrap_or(path);
    if path.is_file() {
        path.parent().unwrap_or(&path).to_path_buf()
    } else {
        path
    }
}

pub fn resolve_dep(
    req: &str,
    decks_dir: Option<&Path>,
    requiring_dir: Option<&Path>,
) -> Option<PathBuf> {
    let stem = Path::new(req)
        .with_extension("")
        .to_string_lossy()
        .into_owned();
    let with_md = |p: &Path| -> PathBuf { p.with_extension("md") };
    let mut candidates = vec![PathBuf::from(req), with_md(Path::new(&stem))];
    for dir in [requiring_dir, decks_dir].into_iter().flatten() {
        candidates.push(dir.join(req));
        candidates.push(with_md(&dir.join(&stem)));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// ADR 0026: a pure function of the text, never of which files exist. Id-mode
/// is stricter than the id grammar (canonical 26 only), so natural `deck-*`
/// filenames (`deck-basics`) stay referenceable by filename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiresMode {
    DeckId,
    WrongTypeCardId,
    Filename,
}

pub fn classify_require(value: &str) -> RequiresMode {
    if value
        .strip_prefix("deck-")
        .is_some_and(crate::token::is_canonical)
    {
        RequiresMode::DeckId
    } else if value
        .strip_prefix("card-")
        .is_some_and(crate::token::is_canonical)
    {
        RequiresMode::WrongTypeCardId
    } else {
        RequiresMode::Filename
    }
}

/// Rename-proof resolution: the deck whose frontmatter `id:` matches, wherever
/// its file lives in the searched directories.
pub fn resolve_dep_by_id(
    deck_id: &str,
    decks_dir: Option<&Path>,
    requiring_dir: Option<&Path>,
) -> Option<PathBuf> {
    let mut seen = HashSet::new();
    for dir in [requiring_dir, decks_dir].into_iter().flatten() {
        if !seen.insert(dir.to_path_buf()) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        paths.sort();
        for path in paths {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if parser::deck_identity(&text).ok().flatten().as_deref() == Some(deck_id) {
                return Some(path);
            }
        }
    }
    None
}

fn resolve_require(
    req: &str,
    decks_dir: Option<&Path>,
    requiring_dir: Option<&Path>,
) -> Option<PathBuf> {
    match classify_require(req) {
        RequiresMode::DeckId => resolve_dep_by_id(req, decks_dir, requiring_dir),
        // A pasted card id is never a prerequisite; it resolves to nothing.
        RequiresMode::WrongTypeCardId => None,
        RequiresMode::Filename => resolve_dep(req, decks_dir, requiring_dir),
    }
}

pub fn is_locked(deck: &Deck, decks_dir: Option<&Path>, store: &Store) -> bool {
    fn prereqs_finished(
        deck: &Deck,
        decks_dir: Option<&Path>,
        store: &Store,
        visited: &mut HashSet<PathBuf>,
    ) -> bool {
        for req in &deck.requires {
            let Some(path) = resolve_require(req, decks_dir, deck.path.parent()) else {
                continue; // missing prerequisite: don't lock on it
            };
            let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if !visited.insert(key) {
                continue; // already checked, or a cycle: stop recursing
            }
            let Ok(prereq) = Deck::load(&path) else {
                continue; // unreadable prerequisite: don't lock on it
            };
            if prereq.has_exam() && prereq.state(store) != DeckState::Finished {
                return false;
            }
            if !prereqs_finished(&prereq, decks_dir, store, visited) {
                return false;
            }
        }
        true
    }
    !prereqs_finished(deck, decks_dir, store, &mut HashSet::new())
}

pub fn nongating_prerequisites(deck: &Deck) -> Vec<String> {
    if !deck.has_exam() {
        return Vec::new();
    }
    let dir = deck.path.parent();
    let mut out = Vec::new();
    for req in &deck.requires {
        let sourceless = resolve_require(req, dir, dir)
            .and_then(|path| Deck::load(&path).ok())
            .is_some_and(|prereq| !prereq.has_exam());
        if sourceless {
            out.push(req.clone());
        }
    }
    out
}

pub fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

pub fn dependents(target: &Path) -> Vec<String> {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let target = canon(target);
    let mut names = Vec::new();
    let Some(member_dir) = target.parent() else {
        return names;
    };
    let Ok(entries) = std::fs::read_dir(member_dir) else {
        return names;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(deck) = Deck::load(&path) else {
            continue;
        };
        let requires_target = deck.requires.iter().any(|req| {
            resolve_require(req, Some(member_dir), path.parent())
                .is_some_and(|dep| canon(&dep) == target)
        });
        if requires_target {
            names.push(deck.subject);
        }
    }
    names.sort();
    names
}

fn image_base_dir(deck_path: &Path) -> PathBuf {
    crate::workspace::content_root(deck_path)
}

fn resolve_image(base: &Path, image: PathBuf) -> PathBuf {
    if image.is_absolute() {
        image
    } else {
        base.join(image)
    }
}

pub(crate) fn write_deck_text(path: &Path, text: &str) -> Result<(), DeckError> {
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };
    let tmp = path.with_extension("md.tmp");
    crate::fsio::replace_file(&tmp, path, text.as_bytes()).map_err(io_err)?;
    Ok(())
}

// Parse knowledge here means a fenced "## " inside an answer is never mistaken for a card front.
fn front_lines_of(path: &Path, text: &str) -> Result<Vec<usize>, DeckError> {
    parser::card_front_lines(text).map_err(|source| DeckError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

pub fn append_note(path: &Path, front_line: usize, notes: &[String]) -> Result<(), DeckError> {
    if notes.is_empty() {
        return Ok(());
    }
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };
    let text = std::fs::read_to_string(path).map_err(io_err)?;
    // A `>` line after a table row is a parse error, so writing one would
    // make the deck unloadable; refuse with the file untouched.
    let parsed = parser::parse("deck.md", &text).map_err(|source| DeckError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    if parsed
        .tables
        .iter()
        .any(|table| table.rows.iter().any(|row| row.line == front_line))
    {
        return Err(DeckError::TableRowNote {
            path: path.to_path_buf(),
            line: front_line,
        });
    }
    // Block boundaries: plain card fronts plus each table's HEADER line; a
    // row line as boundary would land the note inside the table block.
    let row_lines: std::collections::HashSet<usize> = parsed
        .tables
        .iter()
        .flat_map(|table| table.rows.iter().map(|row| row.line))
        .collect();
    let mut fronts: Vec<usize> = parsed
        .cards
        .iter()
        .map(|card| card.line)
        .filter(|line| !row_lines.contains(line))
        .chain(parsed.tables.iter().map(|table| table.line))
        .collect();
    fronts.sort_unstable();
    fronts.dedup();
    let new_text = insert_note_lines(&text, &fronts, front_line, notes);
    write_deck_text(path, &new_text)
}

pub fn append_cards(path: &Path, cards: &str) -> Result<(), DeckError> {
    let cards = cards.trim_end();
    if cards.is_empty() {
        return Ok(());
    }
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };

    let existing = std::fs::read_to_string(path).map_err(io_err)?;
    let mut new_text = existing.trim_end().to_string();
    if !new_text.is_empty() {
        new_text.push_str("\n\n");
    }
    new_text.push_str(cards);
    new_text.push('\n');
    write_deck_text(path, &new_text)
}

pub fn set_trace_checkpoints(path: &Path, cards: &str) -> Result<(), DeckError> {
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };
    let existing = std::fs::read_to_string(path).map_err(io_err)?;
    let new_text = trace_checkpoint_text(path, &existing, cards)?;
    write_deck_text(path, &new_text)
}

pub fn trace_checkpoint_text(
    path: &Path,
    existing: &str,
    cards: &str,
) -> Result<String, DeckError> {
    let fronts = front_lines_of(path, existing)?;
    Ok(replace_after_header(existing, &fronts, cards))
}

fn replace_after_header(text: &str, fronts: &[usize], cards: &str) -> String {
    let cards = cards.trim_end();
    let header: Vec<&str> = match fronts.first() {
        Some(&first) => text.lines().take(first.saturating_sub(1)).collect(),
        None => text.lines().collect(),
    };
    let header = header.join("\n");
    let header = header.trim_end();
    let mut out = String::new();
    if !header.is_empty() {
        out.push_str(header);
        if !cards.is_empty() {
            out.push_str("\n\n");
        }
    }
    out.push_str(cards);
    out.push('\n');
    out
}

pub struct AtRewrite {
    pub at: String,
    pub fingerprint: Option<u64>,
    pub asset: Option<String>,
    pub line: usize,
}

pub fn set_source_citations(path: &Path, ats: &[AtRewrite]) -> Result<(), DeckError> {
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };
    let existing = std::fs::read_to_string(path).map_err(io_err)?;
    let (new_text, rewritten) = rewrite_source_citations(&existing, ats);
    if rewritten != ats.len() {
        return Err(io_err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("rewrote {rewritten} of {} source citations", ats.len()),
        )));
    }
    write_deck_text(path, &new_text)
}

fn at_indent(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let body = trimmed.strip_prefix("<!--")?.strip_suffix("-->")?;
    let (key, _value) = body.split_once(':')?;
    key.trim()
        .eq_ignore_ascii_case("at")
        .then(|| &line[..line.len() - trimmed.len()])
}

fn format_at_rewrite(indent: &str, rewrite: &AtRewrite) -> String {
    let fields = crate::source::LocatorFields {
        at: rewrite.at.clone(),
        fingerprint: rewrite
            .fingerprint
            .map(crate::source::format_locator_fingerprint),
        asset: rewrite.asset.clone(),
    };
    format!(
        "{indent}<!-- {} -->",
        crate::source::format_locator_fields(&fields)
    )
}

fn rewrite_source_citations(text: &str, ats: &[AtRewrite]) -> (String, usize) {
    let mut rewritten = 0;
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let lineno = index + 1;
        if let Some(rewrite) = ats.get(rewritten)
            && rewrite.line == lineno
            && let Some(indent) = at_indent(line)
        {
            out.push(format_at_rewrite(indent, rewrite));
            rewritten += 1;
        } else {
            out.push(line.to_string());
        }
    }
    let mut joined = out.join("\n");
    if text.ends_with('\n') && !joined.ends_with('\n') {
        joined.push('\n');
    }
    (joined, rewritten)
}

pub fn with_sources(text: &str, sources: &[String]) -> Result<String, DeckError> {
    let parsed = parser::parse("deck.md", text).map_err(|source| DeckError::Parse {
        path: PathBuf::from("deck.md"),
        source,
    })?;
    let Some((open, close)) = parsed.frontmatter_span else {
        return Err(DeckError::Io {
            path: PathBuf::from("deck.md"),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "generated deck has no YAML frontmatter",
            ),
        });
    };
    let mut inserted = false;
    let mut skipping = false;
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        if line_number > open && line_number < close {
            if let Some(key) = top_level_yaml_key(line) {
                skipping = key == "source";
                if skipping && !inserted {
                    push_source_lines(&mut out, sources);
                    inserted = true;
                }
                if skipping {
                    continue;
                }
            }
            if skipping {
                continue;
            }
        }
        if line_number == close && !inserted {
            push_source_lines(&mut out, sources);
            inserted = true;
        }
        out.push(line.to_string());
    }
    let mut joined = out.join("\n");
    if text.ends_with('\n') {
        joined.push('\n');
    }
    Ok(joined)
}

pub fn with_added_source(text: &str, value: &str) -> Result<String, DeckError> {
    let parsed = parser::parse("deck.md", text).map_err(|source| DeckError::Parse {
        path: PathBuf::from("deck.md"),
        source,
    })?;
    let mut sources = parsed.frontmatter.source.clone();
    if sources.iter().any(|existing| existing == value) {
        return Ok(text.to_string());
    }
    sources.push(value.to_string());
    with_sources(text, &sources)
}

fn push_source_lines(out: &mut Vec<String>, sources: &[String]) {
    match sources {
        [] => {}
        [only] => out.push(format!("source: {}", parser::yaml_quote(only))),
        many => {
            out.push("source:".to_string());
            for source in many {
                out.push(format!("  - {}", parser::yaml_quote(source)));
            }
        }
    }
}

pub(crate) fn rewrite_frozen_assets(
    text: &str,
    frontmatter_span: Option<crate::parser::LineSpan>,
    source: Option<&str>,
    ats: &[AtRewrite],
    replacements: &[(Range<usize>, String)],
) -> Result<String, DeckError> {
    let text = replace_ranges(text, replacements)?;
    let Some((open, close)) = frontmatter_span else {
        return Ok(rewrite_source_citations(&text, ats).0);
    };
    let mut inserted = source.is_none();
    let mut skipping = false;
    let mut at_index = 0;
    let mut out: Vec<String> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        let in_frontmatter = line_number > open && line_number < close;
        if in_frontmatter {
            if let Some(key) = top_level_yaml_key(line) {
                // With no replacement source, the deck's own `source:` stays
                // untouched (ADR 0026: freezing never rewrites it).
                skipping = source.is_some() && key == "source";
                if skipping && !inserted {
                    push_replacement_source(&mut out, source);
                    inserted = true;
                }
                if skipping {
                    continue;
                }
            }
            if skipping {
                continue;
            }
            out.push(line.to_string());
            continue;
        }
        if line_number == close && !inserted {
            push_replacement_source(&mut out, source);
            inserted = true;
        }
        if at_index < ats.len()
            && ats[at_index].line == line_number
            && let Some(indent) = at_indent(line)
        {
            out.push(format_at_rewrite(indent, &ats[at_index]));
            at_index += 1;
        } else {
            out.push(line.to_string());
        }
    }
    let mut joined = out.join("\n");
    if text.ends_with('\n') && !joined.ends_with('\n') {
        joined.push('\n');
    }
    if at_index != ats.len() {
        return Err(DeckError::Io {
            path: PathBuf::from("deck.md"),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("rewrote {at_index} of {} source citations", ats.len()),
            ),
        });
    }
    Ok(joined)
}

fn replace_ranges(
    text: &str,
    replacements: &[(Range<usize>, String)],
) -> Result<String, DeckError> {
    let mut replacements = replacements.to_vec();
    replacements.sort_by_key(|(range, _)| range.start);
    let mut previous_end = 0;
    for (range, replacement) in &replacements {
        if range.start < previous_end
            || range.start > range.end
            || range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
            || replacement.contains(['\n', '\r'])
        {
            return Err(DeckError::Io {
                path: PathBuf::from("deck.md"),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid overlapping Markdown replacement",
                ),
            });
        }
        previous_end = range.end;
    }
    let mut out = text.to_string();
    for (range, replacement) in replacements.into_iter().rev() {
        out.replace_range(range, &replacement);
    }
    Ok(out)
}

fn top_level_yaml_key(line: &str) -> Option<&str> {
    if line.starts_with(char::is_whitespace) || line.starts_with('#') {
        return None;
    }
    line.split_once(':')
        .map(|(key, _)| key.trim())
        .filter(|key| !key.is_empty())
}

fn push_replacement_source(out: &mut Vec<String>, source: Option<&str>) {
    if let Some(source) = source {
        out.push(format!("source: {}", parser::yaml_quote(source)));
    }
}

pub fn remove_cards(path: &Path, front_lines: &[usize]) -> Result<(), DeckError> {
    if front_lines.is_empty() {
        return Ok(());
    }
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };
    let text = std::fs::read_to_string(path).map_err(io_err)?;
    let fronts = front_lines_of(path, &text)?;
    let new_text = remove_card_blocks(&text, &fronts, front_lines);
    write_deck_text(path, &new_text)
}

pub fn rewrite_without_cards(
    path: &Path,
    original: &str,
    front_lines: &[usize],
) -> Result<(), DeckError> {
    let fronts = front_lines_of(path, original)?;
    let new_text = remove_card_blocks(original, &fronts, front_lines);
    write_deck_text(path, &new_text)
}

fn remove_card_blocks(text: &str, fronts: &[usize], front_lines: &[usize]) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let targets: std::collections::HashSet<usize> = front_lines.iter().copied().collect();

    let mut drop = vec![false; lines.len()];
    for (i, &front) in fronts.iter().enumerate() {
        if !targets.contains(&front) {
            continue;
        }
        let end = fronts
            .get(i + 1)
            .map(|next| next.saturating_sub(1))
            .unwrap_or(lines.len());
        for lineno in front..=end.min(lines.len()) {
            if lineno >= 1 {
                drop[lineno - 1] = true;
            }
        }
    }

    let kept: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| !drop[*i])
        .map(|(_, line)| *line)
        .collect();
    let mut result = kept.join("\n");
    if text.ends_with('\n') && !result.is_empty() && !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

fn insert_note_lines(text: &str, fronts: &[usize], front_line: usize, notes: &[String]) -> String {
    let lines: Vec<&str> = text.lines().collect();

    let bound = fronts
        .iter()
        .find(|&&f| f > front_line)
        .map(|&f| f.saturating_sub(1))
        .unwrap_or(lines.len())
        .min(lines.len());
    let front_index = front_line.saturating_sub(1);
    let mut last_content = front_index;
    for (i, line) in lines.iter().enumerate().take(bound).skip(front_index + 1) {
        if !line.trim().is_empty() {
            last_content = i;
        }
    }
    // The card's trailing comment markers (`at:` locators, the closing `id:`)
    // stay last: stamping mints at that position, and doctor flags a marker
    // with content after it.
    let content_start = front_index.saturating_add(1);
    let insert_at = (content_start..=last_content)
        .rev()
        .find(|&i| {
            let line = lines[i].trim();
            !(line.starts_with("<!--") && line.ends_with("-->"))
        })
        .map(|i| i.saturating_add(1))
        .unwrap_or(content_start);

    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    for (offset, note) in notes.iter().enumerate() {
        out.insert(insert_at + offset, format!("> {note}"));
    }

    let mut result = out.join("\n");
    if text.ends_with('\n') {
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn workspace_defaults_carry_the_sampling_switch_and_reject_junk() {
        let off = [("sampling".to_string(), "off".to_string())];
        assert_eq!(Some(false), DeckSettings::from_directives(&off).sampling);
        let on = [("sampling".to_string(), "on".to_string())];
        assert_eq!(Some(true), DeckSettings::from_directives(&on).sampling);
        let junk = [("sampling".to_string(), "yes".to_string())];
        assert_eq!(None, DeckSettings::from_directives(&junk).sampling);
    }

    #[test]
    fn workspace_defaults_parse_input_and_order_independently() {
        let input = [("input".to_string(), "type".to_string())];
        let parsed = DeckSettings::from_directives(&input);
        assert_eq!(Some(Input::Type), parsed.input);
        assert_eq!(None, parsed.order);

        let order = [("order".to_string(), "sequential".to_string())];
        let parsed = DeckSettings::from_directives(&order);
        assert_eq!(None, parsed.input);
        assert_eq!(Some(Order::Sequential), parsed.order);
    }

    #[test]
    fn an_unparseable_reveal_value_is_rejected_in_workspace_defaults() {
        let directives = [("reveal".to_string(), "cloze".to_string())];
        assert_eq!(None, DeckSettings::from_directives(&directives).reveal);
        let live = [("reveal".to_string(), "line".to_string())];
        assert_eq!(
            Some(Reveal::Line),
            DeckSettings::from_directives(&live).reveal
        );
    }

    fn write_deck(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    fn empty_store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("p.json")).unwrap();
        (store, dir)
    }

    fn graduate(store: &mut Store, id: &str) {
        store.get_or_insert(id, 0).recall = Some(crate::store::FsrsState {
            state: 2, // Review
            ..Default::default()
        });
    }

    fn learning(store: &mut Store, id: &str) {
        store.get_or_insert(id, 0).recall = Some(crate::store::FsrsState {
            state: 1, // Learning
            ..Default::default()
        });
    }

    fn retire(store: &mut Store, id: &str) {
        store.get_or_insert(id, 0).recall = Some(crate::store::FsrsState {
            state: 2,                // Review (a year-out card has graduated)
            scheduled_days: 100_000, // well past the retirement cap
            ..Default::default()
        });
    }

    fn fronts(text: &str) -> Vec<usize> {
        parser::card_front_lines(text).unwrap()
    }

    #[test]
    fn deck_state_progresses_notstarted_started_finished() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(
            dir.path(),
            "d.md",
            "## a <!-- id: card-q1 -->\n1\n## b <!-- id: card-q2 -->\n2\n",
        );
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();

        assert_eq!(DeckState::NotStarted, deck.state(&store));

        learning(&mut store, &deck.cards[0].id().unwrap());
        assert_eq!(DeckState::Started, deck.state(&store));

        for card in &deck.cards {
            graduate(&mut store, &card.id().unwrap());
        }
        assert_eq!(DeckState::Finished, deck.state(&store));
    }

    #[test]
    fn sourced_deck_is_examdue_until_mastered() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(
            dir.path(),
            "d.md",
            "---\nformat-version: 1\nid: \"deck-d1\"\nsource: https://x\n---\n## a <!-- id: card-q1 -->\n1\n",
        );
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();

        retire(&mut store, &deck.cards[0].id().unwrap());
        assert_eq!(DeckState::ExamDue, deck.state(&store));

        store.set_deck_mastered(deck.deck_token.as_deref().unwrap(), 1);
        assert_eq!(DeckState::Finished, deck.state(&store));
    }

    #[test]
    fn a_sourced_deck_is_examdue_once_every_card_graduates() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(
            dir.path(),
            "d.md",
            "---\nsource: https://x\n---\n## a <!-- id: card-q1 -->\n1\n## b <!-- id: card-q2 -->\n2\n",
        );
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();

        graduate(&mut store, &deck.cards[0].id().unwrap());
        learning(&mut store, &deck.cards[1].id().unwrap());
        assert_eq!(DeckState::Started, deck.state(&store));

        graduate(&mut store, &deck.cards[1].id().unwrap());
        assert_eq!(DeckState::ExamDue, deck.state(&store));
    }

    #[test]
    fn a_sourceless_deck_finishes_once_every_card_graduates() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "d.md", "## a <!-- id: card-q1 -->\n1\n");
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();

        graduate(&mut store, &deck.cards[0].id().unwrap());
        assert_eq!(DeckState::Finished, deck.state(&store));
    }

    #[test]
    fn a_deck_still_learning_a_card_is_only_started() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "d.md", "## a <!-- id: card-q1 -->\n1\n");
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();
        learning(&mut store, &deck.cards[0].id().unwrap());
        assert_eq!(DeckState::Started, deck.state(&store));
    }

    #[test]
    fn nongating_prerequisites_flags_a_sourceless_required_deck() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "## a\n1\n");
        write_deck(dir.path(), "c.md", "---\nsource: https://x\n---\n## c\n1\n");
        let b_path = write_deck(
            dir.path(),
            "b.md",
            "---\nsource: https://x\nrequires:\n  - a\n  - c\n---\n## b\n1\n",
        );
        let b = Deck::load(&b_path).unwrap();
        assert_eq!(vec!["a".to_string()], nongating_prerequisites(&b));
    }

    #[test]
    fn nongating_prerequisites_empty_when_no_exam_or_prereq_missing() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "a.md", "## a\n1\n");
        let b = write_deck(dir.path(), "b.md", "---\nrequires: a\n---\n## b\n1\n");
        assert!(nongating_prerequisites(&Deck::load(&b).unwrap()).is_empty());
        let c = write_deck(
            dir.path(),
            "c.md",
            "---\nsource: https://x\nrequires: nope\n---\n## c\n1\n",
        );
        assert!(nongating_prerequisites(&Deck::load(&c).unwrap()).is_empty());
    }

    #[test]
    fn passing_the_exam_masters_an_undrilled_deck() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(
            dir.path(),
            "d.md",
            "---\nformat-version: 1\nid: \"deck-d1\"\nsource: https://x\n---\n## a <!-- id: card-q1 -->\n1\n## b <!-- id: card-q2 -->\n2\n",
        );
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();
        assert_eq!(DeckState::NotStarted, deck.state(&store));

        store.set_deck_mastered(deck.deck_token.as_deref().unwrap(), 1);
        assert_eq!(DeckState::Finished, deck.state(&store));
    }

    #[test]
    fn sourceless_deck_finishes_on_drill_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "d.md", "## a <!-- id: card-q1 -->\n1\n");
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();
        retire(&mut store, &deck.cards[0].id().unwrap());
        assert_eq!(DeckState::Finished, deck.state(&store));
    }

    #[test]
    fn dependent_stays_locked_until_sourced_prereq_mastered() {
        let dir = tempfile::tempdir().unwrap();
        let basics = write_deck(
            dir.path(),
            "basics.md",
            "---\nformat-version: 1\nid: \"deck-basics1\"\nsource: https://x\n---\n## a <!-- id: card-q1 -->\n1\n",
        );
        let adv = write_deck(
            dir.path(),
            "advanced.md",
            "---\nrequires: basics\n---\n## x\ny\n",
        );
        let advanced = Deck::load(&adv).unwrap();
        let basics = Deck::load(&basics).unwrap();
        let (mut store, _s) = empty_store();
        let dd = Some(dir.path());

        retire(&mut store, &basics.cards[0].id().unwrap());
        assert_eq!(DeckState::ExamDue, basics.state(&store));
        assert!(is_locked(&advanced, dd, &store));

        store.set_deck_mastered(basics.deck_token.as_deref().unwrap(), 1);
        assert!(!is_locked(&advanced, dd, &store));
    }

    #[test]
    fn dependents_lists_requiring_decks() {
        let dir = tempfile::tempdir().unwrap();
        let basics = write_deck(dir.path(), "basics.md", "## a\n1\n");
        write_deck(
            dir.path(),
            "advanced.md",
            "---\nrequires: basics\n---\n## x\ny\n",
        );
        write_deck(
            dir.path(),
            "expert.md",
            "---\nrequires: advanced\n---\n## z\nw\n",
        );
        write_deck(dir.path(), "unrelated.md", "## q\nr\n");

        let deps = dependents(&basics);
        assert_eq!(vec!["advanced.md"], deps);
    }

    #[test]
    fn dependents_scan_the_workspace_member_directory() {
        let dir = tempfile::tempdir().unwrap();
        let members = dir.path().join("decks");
        std::fs::create_dir(&members).unwrap();
        std::fs::write(dir.path().join(crate::workspace::MANIFEST), "").unwrap();
        let basics = write_deck(&members, "basics.md", "## a\n1\n");
        write_deck(
            &members,
            "advanced.md",
            "---\nrequires: basics\n---\n## x\ny\n",
        );

        assert_eq!(vec!["advanced.md"], dependents(&basics));
    }

    #[test]
    fn append_cards_appends_with_separation_and_parses() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "d.md", "## one <!-- id: card-q1 -->\n1\n");
        append_cards(
            &path,
            "## two <!-- id: card-q2 --> <!-- reveal: line -->\nkey point\n",
        )
        .unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            "## one <!-- id: card-q1 -->\n1\n\n## two <!-- id: card-q2 --> <!-- reveal: line -->\nkey point\n",
            text
        );
        let cards = parser::parse_str("d.md", &text).unwrap();
        assert_eq!(2, cards.len());
        assert_eq!(Some("card-q1"), cards[0].token.as_deref());
    }

    #[test]
    fn set_trace_checkpoints_replaces_cards_keeping_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(
            dir.path(),
            "t.md",
            "---\ntrace: how it works\nsource: .\n---\n\n## old question\nold point\n<!-- at: 1 -->\n",
        );
        set_trace_checkpoints(
            &path,
            "## new q1\np1\n<!-- at: 2 -->\n## new q2\np2\n<!-- at: 3 -->\n",
        )
        .unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("---\ntrace: how it works\nsource: .\n---\n"));
        assert!(!text.contains("old question"));
        assert!(text.contains("## new q1"));
        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some("how it works".to_string()), deck.trace);
        assert_eq!(2, deck.cards.len());
    }

    #[test]
    fn replace_after_header_appends_when_no_cards_yet() {
        let text = "---\ntrace: how it works\nsource: .\n---\n";
        let out = replace_after_header(text, &fronts(text), "## q\np\n");
        assert_eq!("---\ntrace: how it works\nsource: .\n---\n\n## q\np\n", out);
    }

    #[test]
    fn replace_after_header_is_not_fooled_by_a_fenced_heading() {
        let text = "# Preamble\n```\n## not a card\n```\ntail\n\n## real\nold\n";
        let out = replace_after_header(text, &fronts(text), "## new\nfresh\n");
        assert_eq!(
            "# Preamble\n```\n## not a card\n```\ntail\n\n## new\nfresh\n",
            out
        );
    }

    #[test]
    fn empty_deck_is_not_started() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "e.md", "only a comment\n");
        let deck = Deck::load(&path).unwrap();
        let (store, _s) = empty_store();
        assert!(deck.cards.is_empty());
        assert_eq!(DeckState::NotStarted, deck.state(&store));
    }

    #[test]
    fn source_less_prerequisite_never_locks() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(dir.path(), "basics.md", "## a\n1\n");
        let adv = write_deck(
            dir.path(),
            "advanced.md",
            "---\nrequires: basics\n---\n## x\ny\n",
        );
        let advanced = Deck::load(&adv).unwrap();
        let (store, _s) = empty_store();
        let dd = Some(dir.path());

        assert!(!is_locked(&advanced, dd, &store));
    }

    #[test]
    fn lock_sees_through_a_source_less_prereq_to_a_sourced_ancestor() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(
            dir.path(),
            "a.md",
            "---\nformat-version: 1\nid: \"deck-a1\"\nsource: https://x\n---\n## a\n1\n",
        );
        write_deck(
            dir.path(),
            "b.md",
            "---\nrequires: a\n---\n## b <!-- id: card-q1 -->\n2\n",
        );
        let cpath = write_deck(
            dir.path(),
            "c.md",
            "---\nsource: https://y\nrequires: b\n---\n## c\n3\n",
        );
        let c = Deck::load(&cpath).unwrap();
        let a = Deck::load(dir.path().join("a.md")).unwrap();
        let b = Deck::load(dir.path().join("b.md")).unwrap();
        let (mut store, _s) = empty_store();
        let dd = Some(dir.path());

        assert!(is_locked(&c, dd, &store));
        retire(&mut store, &b.cards[0].id().unwrap());
        assert!(is_locked(&c, dd, &store));
        store.set_deck_mastered(a.deck_token.as_deref().unwrap(), 1);
        assert!(!is_locked(&c, dd, &store));
    }

    #[test]
    fn missing_prerequisite_does_not_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "d.md", "---\nrequires: nope\n---\n## a\n1\n");
        let deck = Deck::load(&path).unwrap();
        let (store, _s) = empty_store();
        assert!(!is_locked(&deck, Some(dir.path()), &store));
    }

    const CANONICAL_DECK_ID: &str = "deck-9w2c7x4k1m8q3z5t0v6b2n4d8f";

    #[test]
    fn classify_require_is_a_pure_three_way_split_of_the_text() {
        assert_eq!(RequiresMode::DeckId, classify_require(CANONICAL_DECK_ID));
        assert_eq!(
            RequiresMode::WrongTypeCardId,
            classify_require("card-9w2c7x4k1m8q3z5t0v6b2n4d8f")
        );
        // Natural `deck-*` names fail the canonical-26 test and stay filenames.
        assert_eq!(RequiresMode::Filename, classify_require("deck-basics"));
        assert_eq!(RequiresMode::Filename, classify_require("card-tricks"));
        assert_eq!(RequiresMode::Filename, classify_require("basics"));
        // A `./` escape is always a filename, even when named like an id.
        assert_eq!(
            RequiresMode::Filename,
            classify_require(&format!("./{CANONICAL_DECK_ID}"))
        );
        // Truncated (25) and non-canonical charset (`l`) both stay filenames.
        assert_eq!(
            RequiresMode::Filename,
            classify_require("deck-9w2c7x4k1m8q3z5t0v6b2n4d8")
        );
        assert_eq!(
            RequiresMode::Filename,
            classify_require("deck-lw2c7x4k1m8q3z5t0v6b2n4d8f")
        );
    }

    #[test]
    fn an_id_mode_prerequisite_gates_and_survives_a_rename() {
        let dir = tempfile::tempdir().unwrap();
        let basics_path = write_deck(
            dir.path(),
            "basics.md",
            &format!(
                "---\nformat-version: 1\nid: \"{CANONICAL_DECK_ID}\"\nsource: https://x\n---\n## a <!-- id: card-q1 -->\n1\n"
            ),
        );
        let adv = write_deck(
            dir.path(),
            "advanced.md",
            &format!("---\nrequires: {CANONICAL_DECK_ID}\n---\n## x\ny\n"),
        );
        let advanced = Deck::load(&adv).unwrap();
        let (mut store, _s) = empty_store();
        let dd = Some(dir.path());

        assert!(is_locked(&advanced, dd, &store));

        let renamed = dir.path().join("fundamentals.md");
        std::fs::rename(&basics_path, &renamed).unwrap();
        assert!(
            is_locked(&advanced, dd, &store),
            "the id edge must survive the prerequisite's rename"
        );

        store.set_deck_mastered(CANONICAL_DECK_ID, 1);
        assert!(!is_locked(&advanced, dd, &store));
    }

    #[test]
    fn id_resolution_searches_each_supplied_directory_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(
            dir.path(),
            "renamed.md",
            &format!("---\nformat-version: 1\nid: \"{CANONICAL_DECK_ID}\"\n---\n## q\na\n"),
        );

        assert_eq!(
            Some(path),
            resolve_dep_by_id(CANONICAL_DECK_ID, Some(dir.path()), None)
        );
    }

    #[test]
    fn a_pasted_card_id_prerequisite_never_locks() {
        let dir = tempfile::tempdir().unwrap();
        let adv = write_deck(
            dir.path(),
            "advanced.md",
            "---\nrequires: card-9w2c7x4k1m8q3z5t0v6b2n4d8f\n---\n## x\ny\n",
        );
        let advanced = Deck::load(&adv).unwrap();
        let (store, _s) = empty_store();
        assert!(!is_locked(&advanced, Some(dir.path()), &store));
    }

    #[test]
    fn a_file_named_like_a_required_id_never_shadows_the_id() {
        let dir = tempfile::tempdir().unwrap();
        // A shadowing file that would lock if it resolved as the prerequisite.
        write_deck(
            dir.path(),
            &format!("{CANONICAL_DECK_ID}.md"),
            "---\nsource: https://x\n---\n## a <!-- id: card-q1 -->\n1\n",
        );
        let adv = write_deck(
            dir.path(),
            "advanced.md",
            &format!("---\nrequires: {CANONICAL_DECK_ID}\n---\n## x\ny\n"),
        );
        let advanced = Deck::load(&adv).unwrap();
        let (store, _s) = empty_store();

        // No deck carries the id, so the edge dangles and never locks; the
        // like-named file is not consulted.
        assert!(!is_locked(&advanced, Some(dir.path()), &store));
    }

    #[test]
    fn a_dot_slash_require_reaches_the_file_named_like_an_id() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(
            dir.path(),
            &format!("{CANONICAL_DECK_ID}.md"),
            "---\nsource: https://x\n---\n## a <!-- id: card-q1 -->\n1\n",
        );
        let adv = write_deck(
            dir.path(),
            "advanced.md",
            &format!("---\nrequires: ./{CANONICAL_DECK_ID}\n---\n## x\ny\n"),
        );
        let advanced = Deck::load(&adv).unwrap();
        let (store, _s) = empty_store();

        assert!(is_locked(&advanced, Some(dir.path()), &store));
    }

    #[test]
    fn a_prerequisite_cycle_resolves_locked_instead_of_hanging() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(
            dir.path(),
            "a.md",
            "---\nsource: https://x\nrequires: b\n---\n## a\n1\n",
        );
        write_deck(
            dir.path(),
            "b.md",
            "---\nsource: https://y\nrequires: a\n---\n## b\n2\n",
        );
        let a = Deck::load(dir.path().join("a.md")).unwrap();
        let (store, _s) = empty_store();
        assert!(is_locked(&a, Some(dir.path()), &store));
    }

    #[test]
    fn resolve_dep_strips_any_extension_and_matches_md() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "x").unwrap();
        let found = resolve_dep("notes.md", Some(dir.path()), None).unwrap();
        assert_eq!(dir.path().join("notes.md"), found);
        let found = resolve_dep("notes", Some(dir.path()), None).unwrap();
        assert_eq!(dir.path().join("notes.md"), found);
        let found = resolve_dep("notes.txt", Some(dir.path()), None).unwrap();
        assert_eq!(dir.path().join("notes.md"), found);
    }

    #[test]
    fn load_deck_subject_is_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mydeck.md");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "## front\nback").unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!("mydeck.md", deck.subject);
        assert_eq!(1, deck.cards.len());
        assert_eq!("mydeck.md", &*deck.cards[0].subject);
    }

    #[test]
    fn insert_note_after_existing_card_content() {
        let text = "## one\nback 1\n> old note\n\n## two\nback 2\n";
        let notes = vec!["new a".to_string(), "new b".to_string()];
        let result = insert_note_lines(text, &fronts(text), 1, &notes);
        assert_eq!(
            "## one\nback 1\n> old note\n> new a\n> new b\n\n## two\nback 2\n",
            result
        );
        let cards = parser::parse_str("s.md", &result).unwrap();
        assert_eq!(Some("old note\nnew a\nnew b".to_string()), cards[0].note);
    }

    #[test]
    fn insert_note_on_last_card_without_note() {
        let text = "## one\nback 1\n";
        let result = insert_note_lines(text, &fronts(text), 1, &["note".to_string()]);
        assert_eq!("## one\nback 1\n> note\n", result);
        let cards = parser::parse_str("s.md", &result).unwrap();
        assert_eq!(Some("note".to_string()), cards[0].note);
    }

    #[test]
    fn insert_note_targets_the_right_card() {
        let text = "## one\nback 1\n\n## two\nback 2\n\n## three\nback 3\n";
        let result = insert_note_lines(text, &fronts(text), 4, &["mid".to_string()]);
        let cards = parser::parse_str("s.md", &result).unwrap();
        assert_eq!(None, cards[0].note);
        assert_eq!(Some("mid".to_string()), cards[1].note);
        assert_eq!(None, cards[2].note);
    }

    #[test]
    fn insert_note_is_not_fooled_by_a_fenced_heading() {
        let text = "## one\n```\n## not a card\n```\ntail\n\n## two\nb\n";
        let result = insert_note_lines(text, &fronts(text), 1, &["n".to_string()]);
        assert_eq!(
            "## one\n```\n## not a card\n```\ntail\n> n\n\n## two\nb\n",
            result
        );
    }

    #[test]
    fn append_note_rewrites_the_file_and_card_ids_survive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "## front <!-- id: card-q1 -->\nanswer\n").unwrap();

        let before = Deck::load(&path).unwrap();
        append_note(&path, 1, &["explained".to_string()]).unwrap();
        let after = Deck::load(&path).unwrap();

        assert_eq!(Some("explained".to_string()), after.cards[0].note);
        assert_eq!(before.cards[0].id(), after.cards[0].id());
    }

    #[test]
    fn appending_a_note_to_a_table_row_refuses_loudly_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "| word | meaning | note |\n|---|---|---|\n| one | eins | old | <!-- r:4k2x9w -->\n| two | zwei | | <!-- r:7m3p5q -->\n<!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->\n",
        )
        .unwrap();
        let before = std::fs::read_to_string(&path).unwrap();

        let result = append_note(&path, 3, &["fresh".to_string()]);

        assert!(
            matches!(result, Err(DeckError::TableRowNote { line: 3, .. })),
            "{result:?}"
        );
        assert_eq!(before, std::fs::read_to_string(&path).unwrap());
        Deck::load(&path).expect("persisting a tutor note must not corrupt the table deck");
    }

    #[test]
    fn a_plain_card_still_takes_notes_in_a_deck_that_also_holds_a_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "## q\na\n\n| word | meaning |\n|---|---|\n| one | eins | <!-- r:4k2x9w -->\n<!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->\n",
        )
        .unwrap();

        append_note(&path, 1, &["explained".to_string()]).unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some("explained"), deck.cards[0].note.as_deref());
    }

    #[test]
    fn append_note_lands_before_the_closing_comment_markers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "## front\nanswer\n> old note\n<!-- at: notes.md:1-2 -->\n<!-- id: card-q1 -->\n\n## next\nb\n<!-- id: card-q2 -->\n",
        )
        .unwrap();

        append_note(&path, 1, &["fresh".to_string()]).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            "## front\nanswer\n> old note\n> fresh\n<!-- at: notes.md:1-2 -->\n<!-- id: card-q1 -->\n\n## next\nb\n<!-- id: card-q2 -->\n",
            text,
            "the id marker must stay the card's last line"
        );
    }

    #[test]
    fn remove_card_block_drops_front_back_and_trailing_blank() {
        let text = "## one\nback 1\n> a note\n\n## two\nback 2\n";
        assert_eq!(
            "## two\nback 2\n",
            remove_card_blocks(text, &fronts(text), &[1])
        );
        assert_eq!(
            "## one\nback 1\n> a note\n",
            remove_card_blocks(text, &fronts(text), &[5])
        );
    }

    #[test]
    fn remove_card_block_keeps_header_and_neighbors() {
        let text = "---\nrequires: base\nlink: https://x\n---\n## a\nx\n## b\ny\n## c\nz\n";
        assert_eq!(
            "---\nrequires: base\nlink: https://x\n---\n## a\nx\n## c\nz\n",
            remove_card_blocks(text, &fronts(text), &[7])
        );
    }

    #[test]
    fn remove_card_block_is_not_fooled_by_a_fenced_heading() {
        let text = "## q\n```\n## not a card\n```\n## next\nb\n";
        assert_eq!(
            "## next\nb\n",
            remove_card_blocks(text, &fronts(text), &[1])
        );
    }

    #[test]
    fn remove_multiple_and_stale_line_is_ignored() {
        let text = "## a\nx\n## b\ny\n## c\nz\n";
        assert_eq!(
            "## b\ny\n",
            remove_card_blocks(text, &fronts(text), &[1, 2, 5])
        );
        assert_eq!("", remove_card_blocks(text, &fronts(text), &[1, 3, 5]));
    }

    #[test]
    fn remove_cards_rewrites_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "## one\nback 1\n\n## two\nback 2\n").unwrap();

        remove_cards(&path, &[1]).unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(1, deck.cards.len());
        assert_eq!("two", deck.cards[0].front);
    }

    #[test]
    fn settings_parsed_from_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\nreveal: line\norder: sequential\ndirection: bogus\n---\n## f\nb\n",
        )
        .unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some(Reveal::Line), deck.settings.reveal);
        assert_eq!(Some(Order::Sequential), deck.settings.order);
        // An unparseable value is linted (doctor material), not an error.
        assert_eq!(None, deck.settings.direction);
    }

    #[test]
    fn base_root_derives_from_the_first_local_source() {
        let dir = tempfile::tempdir().unwrap();
        let decks = dir.path().join("decks");
        let source = dir.path().join("project/src/lib.rs");
        std::fs::create_dir_all(&decks).unwrap();
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "// lib\n").unwrap();
        let path = write_deck(
            &decks,
            "d.md",
            &format!("---\nsource: {}\n---\n## f\nb\n", source.to_string_lossy()),
        );

        // ADR 0026: the deck's first local-path source IS the mechanical base;
        // a file source resolves to its directory.
        assert_eq!(
            Some(source.parent().unwrap().canonicalize().unwrap()),
            Deck::load(path).unwrap().base_root()
        );
    }

    #[test]
    fn base_root_resolves_a_relative_source_against_the_content_root() {
        let dir = tempfile::tempdir().unwrap();
        let decks = dir.path().join("decks");
        let source = dir.path().join("source");
        std::fs::create_dir_all(&decks).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        let path = write_deck(&decks, "d.md", "---\nsource: ../source\n---\n## f\nb\n");

        assert_eq!(
            Some(source.canonicalize().unwrap()),
            Deck::load(path).unwrap().base_root()
        );
    }

    #[test]
    fn base_root_skips_url_sources_and_falls_back_to_the_workspace_source() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let source = dir.path().join("source");
        let members = workspace.join(crate::workspace::DECKS);
        std::fs::create_dir_all(&members).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            workspace.join(crate::workspace::MANIFEST),
            "source = \"../source\"\n",
        )
        .unwrap();
        let path = write_deck(
            &members,
            "d.md",
            "---\nsource: https://example.org/page\n---\n## f\nb\n",
        );
        let deck = Deck::load(&path).unwrap();

        assert_eq!(Some(source.canonicalize().unwrap()), deck.base_root());

        let own = write_deck(
            &members,
            "own.md",
            "---\nsource: ../source/file.rs\n---\n## f\nb\n",
        );
        std::fs::write(source.join("file.rs"), "x\n").unwrap();
        // The deck's own local source wins over the workspace source.
        assert_eq!(
            Some(source.canonicalize().unwrap()),
            Deck::load(own).unwrap().base_root()
        );
    }

    #[test]
    fn a_deck_without_any_local_source_has_no_base_root() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(
            dir.path(),
            "d.md",
            "---\nsource: https://example.org/page\n---\n## f\nb\n",
        );
        assert_eq!(None, Deck::load(path).unwrap().base_root());
    }

    #[test]
    fn source_layers_carry_the_workspace_source_without_shadowing() {
        let dir = tempfile::tempdir().unwrap();
        let members = dir.path().join(crate::workspace::DECKS);
        std::fs::create_dir_all(&members).unwrap();
        std::fs::write(
            dir.path().join(crate::workspace::MANIFEST),
            "source = [\"https://ws.example\", \"notes.md\"]\n",
        )
        .unwrap();
        let path = write_deck(&members, "d.md", "---\nsource: own.rs\n---\n## f\nb\n");
        let deck = Deck::load(&path).unwrap();

        assert_eq!(
            SourceLayers {
                own: vec!["own.rs".to_string()],
                workspace: vec!["https://ws.example".to_string(), "notes.md".to_string()],
            },
            deck.source_layers()
        );
        assert!(deck.has_exam());
    }

    #[test]
    fn a_workspace_source_alone_confers_an_exam() {
        let dir = tempfile::tempdir().unwrap();
        let members = dir.path().join(crate::workspace::DECKS);
        std::fs::create_dir_all(&members).unwrap();
        std::fs::write(
            dir.path().join(crate::workspace::MANIFEST),
            "source = \"notes.md\"\n",
        )
        .unwrap();
        let path = write_deck(&members, "d.md", "## f\nb\n");
        assert!(Deck::load(&path).unwrap().has_exam());

        let bare = tempfile::tempdir().unwrap();
        let alone = write_deck(bare.path(), "d.md", "## f\nb\n");
        assert!(!Deck::load(&alone).unwrap().has_exam());
    }

    #[test]
    fn a_local_source_confers_an_exam() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "d.md", "---\nsource: notes.md\n---\n## f\nb\n");
        assert!(Deck::load(&path).unwrap().has_exam());
    }

    #[test]
    fn reference_links_union_deck_and_workspace_url_sources_excluding_files_and_dupes() {
        let dir = tempfile::tempdir().unwrap();
        let members = dir.path().join(crate::workspace::DECKS);
        std::fs::create_dir_all(&members).unwrap();
        std::fs::write(
            dir.path().join(crate::workspace::MANIFEST),
            "source = \"https://ws.example\"\n",
        )
        .unwrap();
        let path = write_deck(
            &members,
            "d.md",
            "---\nlink: https://a.example\nsource:\n  - https://b.example\n  - notes.md\n  - https://a.example\n---\n## f\nb\n",
        );
        let deck = Deck::load(&path).unwrap();
        assert_eq!(
            vec![
                "https://a.example",
                "https://b.example",
                "https://ws.example"
            ],
            deck.reference_links()
        );
        assert_eq!(
            vec![
                "https://b.example",
                "https://a.example",
                "https://ws.example"
            ],
            deck.source_urls()
        );
    }

    #[test]
    fn a_deck_level_strictness_key_is_an_unknown_key_lint() {
        let dir = tempfile::tempdir().unwrap();
        let text = "---\nstrictness: strict\n---\n## f\nb\n";
        let path = write_deck(dir.path(), "d.md", text);

        let parsed = parser::parse("d.md", text).unwrap();
        assert_eq!(
            vec![parser::Lint {
                line: 2,
                kind: parser::LintKind::UnknownKey {
                    key: "strictness".to_string()
                }
            }],
            parsed.lints
        );

        let deck = Deck::load(&path).unwrap();
        assert_eq!(None, deck.settings.exam_strictness);
    }

    #[test]
    fn workspace_defaults_strictness_still_reaches_deck_settings() {
        let text = "---\nstrictness: strict\n---\n## f\nb\n";
        let parsed = parser::parse("d.md", text).unwrap();

        let mut settings = DeckSettings::from_frontmatter(&parsed.frontmatter);
        settings.fill_from(&DeckSettings::from_directives(&[(
            "strictness".to_string(),
            "strict".to_string(),
        )]));
        assert_eq!(Some(Strictness::Strict), settings.exam_strictness);
    }

    #[test]
    fn reveal_directive_parses_and_stamps_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("u.md");
        std::fs::write(&path, "---\nreveal: line\n---\n## steps?\none\ntwo\n").unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some(Reveal::Line), deck.settings.reveal);
        assert_eq!(Some(Reveal::Line), deck.cards[0].reveal);
    }

    #[test]
    fn requires_parsed_from_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "---\nrequires:\n  - basics\n  - x\n---\n## f\nb\n").unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!(vec!["basics".to_string(), "x".to_string()], deck.requires);
    }

    #[test]
    fn card_reveal_is_card_override_else_deck_reveal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\nreveal: flip\n---\n## a <!-- reveal: line -->\nx\n## b\ny\n",
        )
        .unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some(Reveal::Line), deck.cards[0].reveal);
        assert_eq!(Some(Reveal::Flip), deck.cards[1].reveal);
    }

    #[test]
    fn card_input_is_card_override_else_deck_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\ninput: draw\n---\n## a <!-- input: type -->\nx\n## b\ny\n",
        )
        .unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some(Input::Type), deck.cards[0].input);
        assert_eq!(Some(Input::Draw), deck.cards[1].input);
    }

    #[test]
    fn cards_have_no_reveal_without_directives() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "## a\nx\n").unwrap();
        assert_eq!(None, Deck::load(&path).unwrap().cards[0].reveal);
    }

    #[test]
    fn direction_both_expands_to_forward_and_reverse() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "## purported <!-- id: card-q1 --> <!-- direction: both -->\nangeblich\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(2, deck.cards.len());
        assert_eq!("purported", deck.cards[0].front);
        assert_eq!(vec!["angeblich"], deck.cards[0].back);
        assert_eq!("angeblich", deck.cards[1].front);
        assert_eq!(vec!["purported"], deck.cards[1].back);
        assert_eq!(deck.cards[0].line, deck.cards[1].line);
        assert_eq!(Some("card-q1".to_string()), deck.cards[0].id());
        assert_eq!(Some("card-q1-r".to_string()), deck.cards[1].id());
        assert_ne!(deck.cards[0].id(), deck.cards[1].id());
    }

    #[test]
    fn both_directions_keep_a_table_header_as_context() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "## Vocabulary\n| word | meaning |\n|---|---|\n| purported | angeblich | <!-- r:4k2x9w -->\n<!-- direction: both -->\n<!-- id: card-9w2c7x4k1m8q3z5t0v6b2n4d8f -->\n",
        )
        .unwrap();

        let deck = Deck::load(&path).unwrap();

        assert_eq!(2, deck.cards.len());
        for card in &deck.cards {
            assert_eq!(
                vec!["Vocabulary"],
                card.context,
                "the title reaches both halves"
            );
            assert!(!card.context_leads, "a table title labels the front");
        }
    }

    #[test]
    fn direction_reverse_keeps_only_the_swapped_card() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "## q <!-- direction: reverse -->\na\n").unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(1, deck.cards.len());
        assert_eq!("a", deck.cards[0].front);
        assert_eq!(vec!["q"], deck.cards[0].back);
    }

    #[test]
    fn deck_level_direction_applies_to_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "---\ndirection: both\n---\n## a\nb\n").unwrap();
        assert_eq!(2, Deck::load(&path).unwrap().cards.len());
    }

    #[test]
    fn direction_does_not_apply_to_cloze() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\ndirection: both\n---\n## fill\nThe \\blank{x} thing.\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(1, deck.cards.len());
        assert_eq!(Some(0), deck.cards[0].hole);
        assert!(!deck.cards[0].reversed);
    }

    fn resolved_back_images(deck: &Deck) -> Vec<PathBuf> {
        deck.cards[0]
            .images_back
            .iter()
            .map(|i| i.src.clone())
            .collect()
    }

    #[test]
    fn image_src_resolves_relative_to_the_deck_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "## q\nWaxing\n![](sub/moon.png)\n![](crescent.png)\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(
            vec![
                dir.path().join("sub/moon.png"),
                dir.path().join("crescent.png"),
            ],
            resolved_back_images(&deck)
        );
    }

    #[test]
    fn bare_image_src_resolves_next_to_the_deck() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "## q\nWaxing\n![](moon.png)\n").unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(
            vec![dir.path().join("moon.png")],
            resolved_back_images(&deck)
        );
    }

    #[test]
    fn absolute_card_image_is_used_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        // A platform-absolute path: `/elsewhere/...` is not absolute on
        // Windows, where it would get drive-anchored instead of kept as-is.
        let absolute = dir
            .path()
            .join("elsewhere/moon.png")
            .display()
            .to_string()
            .replace('\\', "/");
        std::fs::write(
            &path,
            format!("## q\nWaxing\n![]({absolute})\n![](crescent.png)\n"),
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(
            vec![PathBuf::from(&absolute), dir.path().join("crescent.png")],
            resolved_back_images(&deck)
        );
    }

    #[test]
    fn workspace_defaults_fill_unset_and_reach_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "## purported\nangeblich\n").unwrap();
        let defaults = DeckSettings {
            direction: Some(Direction::Both),
            reveal: Some(Reveal::Line),
            ..Default::default()
        };
        let deck = Deck::load_with_defaults(&path, &defaults).unwrap();
        assert_eq!(2, deck.cards.len());
        assert_eq!(Some(Reveal::Line), deck.cards[0].reveal);
    }

    #[test]
    fn deck_directive_overrides_workspace_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "---\ndirection: forward\n---\n## a\nb\n").unwrap();
        let defaults = DeckSettings {
            direction: Some(Direction::Both),
            ..Default::default()
        };
        let deck = Deck::load_with_defaults(&path, &defaults).unwrap();
        assert_eq!(1, deck.cards.len());
    }

    #[test]
    fn display_name_uses_title_else_stripped_filename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Eng-Sayings.md");
        std::fs::write(&path, "## a\nb\n").unwrap();
        assert_eq!("Eng-Sayings", Deck::load(&path).unwrap().display_name());

        std::fs::write(&path, "# English Sayings\n\n## a\nb\n").unwrap();
        assert_eq!("English Sayings", Deck::load(&path).unwrap().display_name());

        std::fs::write(
            &path,
            "---\ntrace: how a keypress becomes a grade\n---\n## a\nb\n",
        )
        .unwrap();
        assert_eq!(
            "how a keypress becomes a grade",
            Deck::load(&path).unwrap().display_name()
        );
    }

    #[test]
    fn no_directives_yields_empty_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "just a comment\n\n## f\nb\n").unwrap();

        let deck = Deck::load(&path).unwrap();
        assert_eq!(None, deck.settings.reveal);
        assert_eq!(None, deck.settings.input);
        assert_eq!(None, deck.settings.order);
    }

    #[test]
    fn frozen_asset_rewrite_replaces_yaml_lists_and_image_destinations() {
        let text = "---\nsource:\n  - ../README.md\n  - ../src/lib.rs\ntitle: kept\n---\n\n## q\n![diagram](<old image.png>)\na\n<!-- at: src/lib.rs:2 -->\n";
        let parsed = parser::parse("t.md", text).unwrap();
        let image = parser::image_references(text).remove(0);
        let ats = [AtRewrite {
            at: "src/lib.rs:2".into(),
            fingerprint: Some(7),
            asset: Some("sha256-source.rs".into()),
            line: 11,
        }];

        let out = rewrite_frozen_assets(
            text,
            parsed.frontmatter_span,
            Some("../source tree"),
            &ats,
            &[(image.destination, "assets/deck/sha256-image.png".into())],
        )
        .unwrap();

        assert_eq!(
            "---\nsource: \"../source tree\"\ntitle: kept\n---\n\n## q\n![diagram](<assets/deck/sha256-image.png>)\na\n<!-- at: src/lib.rs:2 fingerprint: xxh64-0000000000000007 asset: sha256-source.rs -->\n",
            out
        );
    }

    #[test]
    fn frozen_asset_rewrite_without_a_source_preserves_the_source_yaml_and_stamps_nothing() {
        let text = "---\nsource: ../src\n---\n## q\na\n<!-- at: src/lib.rs:2 -->\n";
        let parsed = parser::parse("t.md", text).unwrap();
        let ats = [AtRewrite {
            at: "src/lib.rs:2".into(),
            fingerprint: Some(7),
            asset: Some("sha256-source.rs".into()),
            line: 6,
        }];

        let out = rewrite_frozen_assets(text, parsed.frontmatter_span, None, &ats, &[]).unwrap();

        assert!(out.contains("source: ../src\n"), "{out}");
        assert!(!out.contains("origin:"), "freezing stamps nothing: {out}");
        assert!(out.contains(
            "<!-- at: src/lib.rs:2 fingerprint: xxh64-0000000000000007 asset: sha256-source.rs -->"
        ));
    }

    #[test]
    fn with_sources_replaces_the_source_list_preserving_other_yaml() {
        let text = "---\nsource:\n  - a.md\n  - b.md\ntrace: t\n---\n## q\na\n";
        let out = with_sources(
            text,
            &["/abs/a.md".to_string(), "https://example.org".to_string()],
        )
        .unwrap();
        assert_eq!(
            "---\nsource:\n  - \"/abs/a.md\"\n  - \"https://example.org\"\ntrace: t\n---\n## q\na\n",
            out
        );
    }

    #[test]
    fn with_added_source_appends_once_and_keeps_an_existing_value() {
        let text = "---\nsource: page.md\n---\n## q\na\n";
        let out = with_added_source(text, "https://example.org/current").unwrap();
        assert!(
            out.contains("source:\n  - \"page.md\"\n  - \"https://example.org/current\"\n"),
            "{out}"
        );
        assert_eq!(
            out,
            with_added_source(&out, "https://example.org/current").unwrap(),
            "adding a duplicate is a no-op"
        );

        let sourceless = "---\ntrace: t\n---\n## q\na\n";
        let out = with_added_source(sourceless, "https://example.org/current").unwrap();
        assert!(
            out.contains("source: \"https://example.org/current\"\n"),
            "{out}"
        );
        assert!(out.contains("trace: t\n"), "{out}");
    }

    #[test]
    fn source_citation_rewrites_preserve_target_indent_and_final_newline() {
        let rewrite = AtRewrite {
            at: "src/lib.rs:2".to_string(),
            fingerprint: Some(7),
            asset: None,
            line: 3,
        };
        let without_newline = "## q\na\n  <!-- at: old.rs:1 -->";
        let expected = "## q\na\n  <!-- at: src/lib.rs:2 fingerprint: xxh64-0000000000000007 -->";

        assert_eq!(
            (expected.to_string(), 1),
            rewrite_source_citations(without_newline, std::slice::from_ref(&rewrite))
        );
        assert_eq!(
            (format!("{expected}\n"), 1),
            rewrite_source_citations(
                &format!("{without_newline}\n"),
                std::slice::from_ref(&rewrite)
            )
        );
    }

    #[test]
    fn frozen_rewrites_preserve_frontmatter_order_and_final_newline() {
        let text = "---\ntitle: kept\nsource: old.md\ntrace: path\n---\n## q\na";
        let parsed = parser::parse("t.md", text).unwrap();
        let expected = "---\ntitle: kept\nsource: \"new.md\"\ntrace: path\n---\n## q\na";

        assert_eq!(
            expected,
            rewrite_frozen_assets(text, parsed.frontmatter_span, Some("new.md"), &[], &[]).unwrap()
        );
        assert_eq!(
            format!("{expected}\n"),
            rewrite_frozen_assets(
                &format!("{text}\n"),
                parsed.frontmatter_span,
                Some("new.md"),
                &[],
                &[]
            )
            .unwrap()
        );

        let sourceless = "---\ntitle: kept\n---\n## q\na\n";
        let parsed = parser::parse("t.md", sourceless).unwrap();
        assert_eq!(
            "---\ntitle: kept\nsource: \"new.md\"\n---\n## q\na\n",
            rewrite_frozen_assets(
                sourceless,
                parsed.frontmatter_span,
                Some("new.md"),
                &[],
                &[]
            )
            .unwrap()
        );
    }

    #[test]
    fn markdown_replacements_accept_boundaries_and_reject_each_invalid_dimension() {
        assert_eq!(
            "AB",
            replace_ranges("ab", &[(0..1, "A".into()), (1..2, "B".into())]).unwrap()
        );
        assert_eq!("ab!", replace_ranges("ab", &[(2..2, "!".into())]).unwrap());

        for (label, text, ranges) in [
            (
                "overlap",
                "abcd",
                vec![(0..3, "x".to_string()), (2..4, "y".to_string())],
            ),
            (
                "reversed",
                "abcd",
                vec![(std::ops::Range { start: 3, end: 2 }, "x".to_string())],
            ),
            ("past end", "abcd", vec![(0..5, "x".to_string())]),
            ("split utf8 start", "é", vec![(1..2, "x".to_string())]),
            ("split utf8 end", "é", vec![(0..1, "x".to_string())]),
            ("line feed", "ab", vec![(0..1, "x\ny".to_string())]),
            ("carriage return", "ab", vec![(0..1, "x\ry".to_string())]),
        ] {
            assert!(
                replace_ranges(text, &ranges).is_err(),
                "{label} must be rejected: {ranges:?}"
            );
        }
    }

    #[test]
    fn top_level_yaml_keys_exclude_each_non_top_level_shape() {
        assert_eq!(Some("source"), top_level_yaml_key("source: x"));
        assert_eq!(None, top_level_yaml_key("  source: nested"));
        assert_eq!(None, top_level_yaml_key("# source: commented"));
        assert_eq!(None, top_level_yaml_key("plain text"));
    }
}
