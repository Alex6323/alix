use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::{
    answer::Input,
    card::{Card, Direction},
    config::Strictness,
    depth::Reveal,
    l1::{self, L1Error},
    session::{self, Order},
    store::Store,
};

#[derive(Debug, Default, Clone)]
pub struct DeckSettings {
    pub reveal: Option<Reveal>,
    pub input: Option<Input>,
    pub order: Option<Order>,
    pub direction: Option<Direction>,
    pub img_dir: Option<PathBuf>,
    pub exam_strictness: Option<Strictness>,
    pub origin: Option<String>,
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
                "img-dir" => settings.img_dir = Some(PathBuf::from(value)),
                "strictness" => settings.exam_strictness = Strictness::parse(value),
                "origin" => {
                    let v = value.trim();
                    if !v.is_empty() {
                        settings.origin = Some(v.to_string());
                    }
                }
                _ => {}
            }
        }
        settings
    }

    pub fn from_frontmatter(frontmatter: &l1::Frontmatter) -> Self {
        Self {
            reveal: frontmatter.reveal,
            input: frontmatter.input,
            order: frontmatter.order,
            direction: frontmatter.direction,
            img_dir: frontmatter.img_dir.clone(),
            // Learner setting: a deck never ships grading rigor.
            exam_strictness: None,
            origin: frontmatter.origin.clone(),
        }
    }

    fn fill_from(&mut self, defaults: &DeckSettings) {
        self.reveal = self.reveal.or(defaults.reveal);
        self.input = self.input.or(defaults.input);
        self.order = self.order.or(defaults.order);
        self.direction = self.direction.or(defaults.direction);
        self.img_dir = self.img_dir.clone().or_else(|| defaults.img_dir.clone());
        self.exam_strictness = self.exam_strictness.or(defaults.exam_strictness);
        self.origin = self.origin.clone().or_else(|| defaults.origin.clone());
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
        source: L1Error,
    },
    #[error("{path}: file name is not valid UTF-8")]
    InvalidFileName { path: PathBuf },
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
        let l1deck = l1::parse_l1(&subject, &text).map_err(|source| DeckError::Parse {
            path: path.clone(),
            source,
        })?;
        let links = l1deck.frontmatter.link.clone();
        let requires = l1deck.frontmatter.requires.clone();
        let sources = l1deck.frontmatter.source.clone();
        let title = l1deck.title.clone();
        let trace = l1deck.frontmatter.trace.clone();
        let deck_token = l1deck.deck_token.clone();
        let mut settings = DeckSettings::from_frontmatter(&l1deck.frontmatter);
        let mut cards = l1deck.cards;
        settings.fill_from(defaults);
        for card in &mut cards {
            card.reveal = card.reveal.or(settings.reveal);
            card.input = card.input.or(settings.input);
        }
        // No filesystem check here: a missing image must not stop the deck from loading.
        let base_dir = image_base_dir(&path, settings.img_dir.as_deref());
        for card in &mut cards {
            card.image = card.image.take().map(|p| resolve_image(&base_dir, p));
            card.image_back = card.image_back.take().map(|p| resolve_image(&base_dir, p));
        }
        let mut expanded = Vec::with_capacity(cards.len());
        for card in cards {
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
            trace,
        })
    }

    pub fn is_trace(&self) -> bool {
        self.trace.is_some()
    }

    pub fn has_exam(&self) -> bool {
        self.is_trace() || !self.sources.is_empty()
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
        if store.deck_mastered(&self.subject) {
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
            .all(|c| c.id().and_then(|id| store.get(&id)).is_none())
        {
            DeckState::NotStarted
        } else {
            DeckState::Started
        }
    }

    pub fn reference_links(&self) -> Vec<String> {
        let mut out = self.links.clone();
        for src in &self.sources {
            if is_url(src) && !out.contains(src) {
                out.push(src.clone());
            }
        }
        out
    }

    pub fn source_root(&self) -> Option<PathBuf> {
        if let Some(origin) = &self.settings.origin {
            return Some(PathBuf::from(origin));
        }
        let deck_dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        // A bare Deck::load has no settings.origin yet, so recover it from the workspace manifest
        // directly.
        if self.is_frozen()
            && let Ok(ws) = crate::workspace::Workspace::load(deck_dir)
            && let Some(origin) = ws.settings.origin
        {
            return Some(PathBuf::from(origin));
        }
        crate::trace::project_root(&self.sources, deck_dir)
    }

    pub fn is_frozen(&self) -> bool {
        self.sources
            .first()
            .is_some_and(|s| s == crate::trace::SNAPSHOT_DIR)
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

pub fn is_locked(deck: &Deck, decks_dir: Option<&Path>, store: &Store) -> bool {
    fn prereqs_finished(
        deck: &Deck,
        decks_dir: Option<&Path>,
        store: &Store,
        visited: &mut HashSet<PathBuf>,
    ) -> bool {
        for req in &deck.requires {
            let Some(path) = resolve_dep(req, decks_dir, deck.path.parent()) else {
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
        let sourceless = resolve_dep(req, dir, dir)
            .and_then(|path| Deck::load(&path).ok())
            .is_some_and(|prereq| !prereq.has_exam());
        if sourceless {
            out.push(req.clone());
        }
    }
    out
}

pub(crate) fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

pub fn dependents(target: &Path, decks_dir: &Path) -> Vec<String> {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let target = canon(target);
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir(decks_dir) else {
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
            resolve_dep(req, Some(decks_dir), path.parent())
                .is_some_and(|dep| canon(&dep) == target)
        });
        if requires_target {
            names.push(deck.subject);
        }
    }
    names.sort();
    names
}

fn image_base_dir(deck_path: &Path, img_dir: Option<&Path>) -> PathBuf {
    let deck_dir = deck_path.parent().unwrap_or_else(|| Path::new("."));
    match img_dir {
        Some(dir) if dir.is_absolute() => dir.to_path_buf(),
        Some(dir) => deck_dir.join(dir),
        None => deck_dir.to_path_buf(),
    }
}

fn resolve_image(base: &Path, image: PathBuf) -> PathBuf {
    if image.is_absolute() {
        image
    } else {
        base.join(image)
    }
}

fn write_deck_text(path: &Path, text: &str) -> Result<(), DeckError> {
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, text).map_err(io_err)?;
    std::fs::rename(&tmp, path).map_err(io_err)?;
    Ok(())
}

// Parse knowledge here means a fenced "## " inside an answer is never mistaken for a card front.
fn front_lines_of(path: &Path, text: &str) -> Result<Vec<usize>, DeckError> {
    l1::card_front_lines(text).map_err(|source| DeckError::Parse {
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
    let fronts = front_lines_of(path, &text)?;
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
    pub origin: Option<String>,
}

pub fn set_trace_snapshot(
    path: &Path,
    source: &str,
    origin: Option<&str>,
    ats: &[AtRewrite],
) -> Result<(), DeckError> {
    let io_err = |source| DeckError::Io {
        path: path.to_path_buf(),
        source,
    };
    let existing = std::fs::read_to_string(path).map_err(io_err)?;
    let span = l1::parse_l1("deck.md", &existing)
        .map_err(|source| DeckError::Parse {
            path: path.to_path_buf(),
            source,
        })?
        .frontmatter_span;
    let new_text = rewrite_trace_snapshot(&existing, span, source, origin, ats);
    write_deck_text(path, &new_text)
}

// A literal <!-- at: --> inside a code fence would be misread here; tool-generated decks never
// produce one.
fn rewrite_trace_snapshot(
    text: &str,
    frontmatter_span: Option<crate::l1::LineSpan>,
    source: &str,
    origin: Option<&str>,
    ats: &[AtRewrite],
) -> String {
    fn at_indent(line: &str) -> Option<&str> {
        let trimmed = line.trim_start();
        let body = trimmed.strip_prefix("<!--")?.strip_suffix("-->")?;
        let (key, _value) = body.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case("at")
            .then(|| &line[..line.len() - trimmed.len()])
    }
    fn is_source_key(line: &str) -> bool {
        line.strip_prefix("source")
            .is_some_and(|rest| rest.trim_start().starts_with(':'))
    }

    let mut source_replaced = false;
    let mut at_i = 0;
    let mut out: Vec<String> = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let lineno = idx + 1;
        let in_frontmatter =
            frontmatter_span.is_some_and(|(open, close)| lineno > open && lineno < close);
        if in_frontmatter && !source_replaced && is_source_key(line) {
            out.push(format!("source: {source}"));
            if let Some(origin) = origin {
                out.push(format!("origin: {origin}"));
            }
            source_replaced = true;
        } else if !in_frontmatter
            && at_i < ats.len()
            && let Some(indent) = at_indent(line)
        {
            match &ats[at_i].origin {
                Some(o) => out.push(format!("{indent}<!-- at: {} from {o} -->", ats[at_i].at)),
                None => out.push(format!("{indent}<!-- at: {} -->", ats[at_i].at)),
            }
            at_i += 1;
        } else {
            out.push(line.to_string());
        }
    }
    let mut joined = out.join("\n");
    if text.ends_with('\n') && !joined.ends_with('\n') {
        joined.push('\n');
    }
    joined
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

    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    for (offset, note) in notes.iter().enumerate() {
        out.insert(last_content + 1 + offset, format!("> {note}"));
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
        l1::card_front_lines(text).unwrap()
    }

    #[test]
    fn deck_state_progresses_notstarted_started_finished() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(
            dir.path(),
            "d.md",
            "## a <!-- id: q1 -->\n1\n## b <!-- id: q2 -->\n2\n",
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
            "---\nsource: https://x\n---\n## a <!-- id: q1 -->\n1\n",
        );
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();

        retire(&mut store, &deck.cards[0].id().unwrap());
        assert_eq!(DeckState::ExamDue, deck.state(&store));

        store.set_deck_mastered(&deck.subject, 1);
        assert_eq!(DeckState::Finished, deck.state(&store));
    }

    #[test]
    fn a_sourced_deck_is_examdue_once_every_card_graduates() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(
            dir.path(),
            "d.md",
            "---\nsource: https://x\n---\n## a <!-- id: q1 -->\n1\n## b <!-- id: q2 -->\n2\n",
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
        let path = write_deck(dir.path(), "d.md", "## a <!-- id: q1 -->\n1\n");
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();

        graduate(&mut store, &deck.cards[0].id().unwrap());
        assert_eq!(DeckState::Finished, deck.state(&store));
    }

    #[test]
    fn a_deck_still_learning_a_card_is_only_started() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "d.md", "## a <!-- id: q1 -->\n1\n");
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
            "---\nsource: https://x\n---\n## a <!-- id: q1 -->\n1\n## b <!-- id: q2 -->\n2\n",
        );
        let deck = Deck::load(&path).unwrap();
        let (mut store, _s) = empty_store();
        assert_eq!(DeckState::NotStarted, deck.state(&store));

        store.set_deck_mastered(&deck.subject, 1);
        assert_eq!(DeckState::Finished, deck.state(&store));
    }

    #[test]
    fn sourceless_deck_finishes_on_drill_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "d.md", "## a <!-- id: q1 -->\n1\n");
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
            "---\nsource: https://x\n---\n## a <!-- id: q1 -->\n1\n",
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

        store.set_deck_mastered(&basics.subject, 1);
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

        let deps = dependents(&basics, dir.path());
        assert_eq!(vec!["advanced.md"], deps);
    }

    #[test]
    fn append_cards_appends_with_separation_and_parses() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(dir.path(), "d.md", "## one <!-- id: q1 -->\n1\n");
        append_cards(
            &path,
            "## two <!-- id: q2 --> <!-- reveal: line -->\nkey point\n",
        )
        .unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            "## one <!-- id: q1 -->\n1\n\n## two <!-- id: q2 --> <!-- reveal: line -->\nkey point\n",
            text
        );
        let cards = l1::parse_str("d.md", &text).unwrap();
        assert_eq!(2, cards.len());
        assert_eq!(Some("q1"), cards[0].token.as_deref());
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
        write_deck(dir.path(), "a.md", "---\nsource: https://x\n---\n## a\n1\n");
        write_deck(
            dir.path(),
            "b.md",
            "---\nrequires: a\n---\n## b <!-- id: q1 -->\n2\n",
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
        store.set_deck_mastered(&a.subject, 1);
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
        let cards = l1::parse_str("s.md", &result).unwrap();
        assert_eq!(Some("old note\nnew a\nnew b".to_string()), cards[0].note);
    }

    #[test]
    fn insert_note_on_last_card_without_note() {
        let text = "## one\nback 1\n";
        let result = insert_note_lines(text, &fronts(text), 1, &["note".to_string()]);
        assert_eq!("## one\nback 1\n> note\n", result);
        let cards = l1::parse_str("s.md", &result).unwrap();
        assert_eq!(Some("note".to_string()), cards[0].note);
    }

    #[test]
    fn insert_note_targets_the_right_card() {
        let text = "## one\nback 1\n\n## two\nback 2\n\n## three\nback 3\n";
        let result = insert_note_lines(text, &fronts(text), 4, &["mid".to_string()]);
        let cards = l1::parse_str("s.md", &result).unwrap();
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
        std::fs::write(&path, "## front <!-- id: q1 -->\nanswer\n").unwrap();

        let before = Deck::load(&path).unwrap();
        append_note(&path, 1, &["explained".to_string()]).unwrap();
        let after = Deck::load(&path).unwrap();

        assert_eq!(Some("explained".to_string()), after.cards[0].note);
        assert_eq!(before.cards[0].id(), after.cards[0].id());
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
    fn origin_cascades_workspace_then_deck() {
        let mut deck =
            DeckSettings::from_directives(&[("origin".to_string(), "/deck".to_string())]);
        deck.fill_from(&DeckSettings::from_directives(&[(
            "origin".to_string(),
            "/ws".to_string(),
        )]));
        assert_eq!(Some("/deck".to_string()), deck.origin);
        let mut bare = DeckSettings::default();
        bare.fill_from(&DeckSettings::from_directives(&[(
            "origin".to_string(),
            "/ws".to_string(),
        )]));
        assert_eq!(Some("/ws".to_string()), bare.origin);
    }

    #[test]
    fn reference_links_union_url_sources_excluding_files_and_dupes() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_deck(
            dir.path(),
            "d.md",
            "---\nlink: https://a.example\nsource:\n  - https://b.example\n  - notes.md\n  - https://a.example\n---\n## f\nb\n",
        );
        let deck = Deck::load(&path).unwrap();
        assert_eq!(
            vec!["https://a.example", "https://b.example"],
            deck.reference_links()
        );
    }

    #[test]
    fn a_deck_level_strictness_key_is_an_unknown_key_lint() {
        let dir = tempfile::tempdir().unwrap();
        let text = "---\nstrictness: strict\n---\n## f\nb\n";
        let path = write_deck(dir.path(), "d.md", text);

        let l1deck = l1::parse_l1("d.md", text).unwrap();
        assert_eq!(
            vec![l1::Lint {
                line: 2,
                kind: l1::LintKind::UnknownKey {
                    key: "strictness".to_string()
                }
            }],
            l1deck.lints
        );

        let deck = Deck::load(&path).unwrap();
        assert_eq!(None, deck.settings.exam_strictness);
    }

    #[test]
    fn workspace_defaults_strictness_still_reaches_deck_settings() {
        let text = "---\nstrictness: strict\n---\n## f\nb\n";
        let l1deck = l1::parse_l1("d.md", text).unwrap();

        let mut settings = DeckSettings::from_frontmatter(&l1deck.frontmatter);
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
            "## purported <!-- id: q1 --> <!-- direction: both -->\nangeblich\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(2, deck.cards.len());
        assert_eq!("purported", deck.cards[0].front);
        assert_eq!(vec!["angeblich"], deck.cards[0].back);
        assert_eq!("angeblich", deck.cards[1].front);
        assert_eq!(vec!["purported"], deck.cards[1].back);
        assert_eq!(deck.cards[0].line, deck.cards[1].line);
        assert_eq!(Some("q1".to_string()), deck.cards[0].id());
        assert_eq!(Some("q1-r".to_string()), deck.cards[1].id());
        assert_ne!(deck.cards[0].id(), deck.cards[1].id());
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
            "---\ndirection: both\n---\n## fill\nThe \\cloze{x} thing.\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(1, deck.cards.len());
        assert_eq!(Some(0), deck.cards[0].hole);
        assert!(!deck.cards[0].reversed);
    }

    #[test]
    fn image_resolves_against_img_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\nimg-dir: /assets/imgs\n---\n## q <!-- img: moon.png -->\nWaxing\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(
            Some(PathBuf::from("/assets/imgs/moon.png")),
            deck.cards[0].image
        );
    }

    #[test]
    fn image_resolves_against_deck_dir_without_img_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(&path, "## q <!-- img: moon.png -->\nWaxing\n").unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(Some(dir.path().join("moon.png")), deck.cards[0].image);
    }

    #[test]
    fn absolute_card_image_is_used_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.md");
        std::fs::write(
            &path,
            "---\nimg-dir: /assets\n---\n## q <!-- img: /elsewhere/moon.png -->\nWaxing\n",
        )
        .unwrap();
        let deck = Deck::load(&path).unwrap();
        assert_eq!(
            Some(PathBuf::from("/elsewhere/moon.png")),
            deck.cards[0].image
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
    fn rewrite_trace_snapshot_repoints_source_origin_and_each_at() {
        let text = "---\ntrace: how X\nsource: ..\n---\n\n## q1\np\n<!-- at: a.rs:90-98 -->\n## q2\np\n<!-- at: b.rs:1 -->\n";
        let span = l1::parse_l1("t.md", text).unwrap().frontmatter_span;
        let ats = [
            AtRewrite {
                at: "01.rs".into(),
                origin: Some("src/a.rs:90-98".into()),
            },
            AtRewrite {
                at: "02.rs".into(),
                origin: Some("src/b.rs:1".into()),
            },
        ];
        let out = rewrite_trace_snapshot(text, span, "assets", Some("/crate"), &ats);
        assert!(out.contains("source: assets\n"), "{out}");
        assert_eq!(1, out.matches("source:").count());
        assert!(out.contains("origin: /crate\n"), "{out}");
        assert!(
            out.contains("<!-- at: 01.rs from src/a.rs:90-98 -->\n"),
            "{out}"
        );
        assert!(
            out.contains("<!-- at: 02.rs from src/b.rs:1 -->\n"),
            "{out}"
        );
        assert!(out.contains("trace: how X\n"));
        assert!(out.ends_with('\n'));
    }
}
