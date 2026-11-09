use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use thiserror::Error;

use super::{
    Binding, DeckSources, DisplayPath, MANIFEST_VERSION, Manifest, ROOTS_VERSION, Roots, Row,
    binding_of, canonical_folder, copy_name, line_count, load_verified,
    manifest::topic_of,
    parse_binding,
    roots::{root_fix, root_issue},
    source_id, stray_copies,
    transaction::{Candidate, CommitReport, commit, save_roots},
};
use crate::{
    config::{BackendKind, ExamConfig},
    deck::{self, Deck},
    exam_prompt::{PreflightError, TopicMeasure, measured_prompt},
    exam_status, locator, parser, state,
    store::{Store, StoreError},
    topic,
    workspace::{self, WorkspaceFiles},
};

#[derive(Debug, Error)]
#[error("results not kept for {}", .topics.join(", "))]
pub struct KeepResultsNotSaved {
    pub topics: Vec<String>,
    #[source]
    pub source: StoreError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddRequest<'a> {
    pub deck: &'a Path,
    pub files: Vec<PathBuf>,
    pub root: Option<(String, PathBuf)>,
    pub key: Option<String>,
    pub topics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddReport {
    pub added: Vec<(String, String)>,
    pub collected: Vec<String>,
    pub over_limit: Vec<PreflightError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveReport {
    pub removed: Vec<(String, String)>,
    pub collected: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReport {
    pub old_id: String,
    pub new_id: String,
    pub path: String,
    pub kept_ranges: Vec<String>,
    pub collected: Vec<String>,
    pub over_limit: Vec<PreflightError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindReport {
    pub over_limit: Vec<PreflightError>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepairReport {
    pub refrozen: Vec<(String, String)>,
    pub missing: Vec<String>,
    pub skipped: Vec<(String, String)>,
    pub kept_ranges: Vec<(String, String)>,
    pub collected: Vec<String>,
    pub over_limit: Vec<PreflightError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasuredTopic {
    pub topic_id: String,
    pub label: String,
    pub measures: Vec<TopicMeasure>,
}

pub fn topic_measures(
    deck: &Deck,
    sources: &DeckSources,
    exam: &ExamConfig,
) -> Result<Vec<MeasuredTopic>, super::SourceError> {
    let mut measured = Vec::new();
    for topic in exam_status::exam_topics(deck, Some(&sources.manifest)) {
        if !topic.sourced {
            continue;
        }
        let texts = sources.texts_of(&topic.topic_id)?;
        let prompt = measured_prompt(&topic, &texts, exam);
        measured.push(MeasuredTopic {
            measures: BackendKind::ALL
                .iter()
                .map(|&kind| TopicMeasure::of(kind, &prompt))
                .collect(),
            topic_id: topic.topic_id,
            label: topic.label,
        });
    }
    Ok(measured)
}

pub fn over_limit(measured: &[MeasuredTopic]) -> Vec<PreflightError> {
    measured
        .iter()
        .flat_map(|topic| {
            topic
                .measures
                .iter()
                .filter(|measure| measure.exceeds())
                .map(|&measure| PreflightError {
                    topic: topic.label.clone(),
                    measure,
                })
        })
        .collect()
}

fn over_limit_of(
    deck: &Deck,
    sources: &DeckSources,
    exam: &ExamConfig,
) -> Result<Vec<PreflightError>> {
    Ok(over_limit(&topic_measures(deck, sources, exam)?))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootChange {
    pub old: Option<PathBuf>,
    pub new: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub topics: Vec<TopicLine>,
    pub sources: Vec<SourceLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicLine {
    pub id: String,
    pub label: String,
    pub source_count: usize,
    pub source_bytes: u64,
    pub measures: Vec<TopicMeasure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLine {
    pub id: String,
    pub bytes: u64,
    pub path: String,
    pub bound: Vec<String>,
    pub drift: Drift,
    pub root_issue: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drift {
    Unchecked,
    Same,
    Changed,
    Missing,
    Linked,
    Unreadable,
}

impl Drift {
    pub fn label(self) -> &'static str {
        match self {
            Drift::Unchecked => "unchecked",
            Drift::Same => "same",
            Drift::Changed => "changed",
            Drift::Missing => "missing",
            Drift::Linked => "linked",
            Drift::Unreadable => "unreadable",
        }
    }
}

struct Opened {
    deck: Deck,
    deck_id: String,
    root: PathBuf,
    dir: PathBuf,
    manifest: Manifest,
    copies: BTreeMap<String, Vec<u8>>,
    roots: Roots,
}

fn open(deck_path: &Path) -> Result<Opened> {
    let Some(root) = workspace::root_for_deck(deck_path) else {
        bail!(
            "`{}` is not a workspace member; only a deck in a workspace's `decks/` folder has \
             sources",
            deck_path.display()
        );
    };
    let deck = Deck::load(deck_path)?;
    let Some(deck_id) = deck.deck_token.clone() else {
        bail!(
            "`{}` has no deck id; run `alix deck init` on it first",
            deck_path.display()
        );
    };
    let duplicates = topic::duplicated_topics(&deck, Some(&root));
    if !duplicates.is_empty() {
        let findings: Vec<String> = duplicates.iter().map(ToString::to_string).collect();
        bail!("{}", findings.join("\n"));
    }
    let dir = WorkspaceFiles::new(&root).sources_for(&deck_id);
    let (manifest, copies) = match load_verified(&root, &deck_id)? {
        Some(sources) => {
            sources.validate_bindings(&deck)?;
            (sources.manifest, sources.copies)
        }
        None => (
            Manifest {
                version: MANIFEST_VERSION,
                sources: BTreeMap::new(),
            },
            BTreeMap::new(),
        ),
    };
    let roots = Roots::load(&dir)?.unwrap_or(Roots {
        version: ROOTS_VERSION,
        roots: BTreeMap::new(),
    });
    Ok(Opened {
        deck,
        deck_id,
        root,
        dir,
        manifest,
        copies,
        roots,
    })
}

struct KeepResults {
    before: BTreeMap<String, String>,
    store: Store,
}

fn open_keep_results(opened: &Opened) -> Result<KeepResults> {
    let before = opened.manifest.fingerprints()?;
    let store = state::open_deck_store(
        &state::UserFiles::new(&opened.root),
        &opened.deck_id,
        &opened.deck.subject,
    )?;
    Ok(KeepResults { before, store })
}

fn keep_topic_results(opened: &Opened, manifest: &Manifest, keep: KeepResults) -> Result<()> {
    let KeepResults { before, mut store } = keep;
    let after = manifest.fingerprints()?;
    let personal = crate::assemble::personal_cards(&opened.deck).cards;
    let current = exam_status::current_stamp(&opened.deck, &personal, Some(manifest));
    let rekeyed = store.keep_results(&opened.deck_id, &before, &after, &current);
    if rekeyed.is_empty() {
        return Ok(());
    }
    store.save().map_err(|source| KeepResultsNotSaved {
        topics: topic_labels(&opened.deck, manifest, rekeyed),
        source,
    })?;
    Ok(())
}

fn topic_labels(deck: &Deck, manifest: &Manifest, topics: Vec<String>) -> Vec<String> {
    let labels: BTreeMap<String, String> = exam_status::exam_topics(deck, Some(manifest))
        .into_iter()
        .map(|topic| (topic.topic_id, topic.label))
        .collect();
    topics
        .into_iter()
        .map(|id| labels.get(&id).cloned().unwrap_or(id))
        .collect()
}

fn finish(
    opened: &Opened,
    manifest: Manifest,
    new_copies: Vec<(String, Vec<u8>)>,
    roots: Option<Roots>,
) -> Result<CommitReport> {
    let candidate = DeckSources {
        dir: opened.dir.clone(),
        manifest,
        copies: BTreeMap::new(),
    };
    candidate.manifest.validate()?;
    candidate.validate_bindings(&opened.deck)?;
    Ok(commit(
        &opened.dir,
        Candidate {
            manifest: candidate.manifest,
            new_copies,
            roots,
        },
    )?)
}

fn unique_prefix<'a>(candidates: Vec<&'a str>, prefix: &str, noun: &str) -> Result<&'a str> {
    if let Some(exact) = candidates.iter().find(|candidate| **candidate == prefix) {
        return Ok(exact);
    }
    let matches: Vec<&str> = candidates
        .into_iter()
        .filter(|candidate| !prefix.is_empty() && candidate.starts_with(prefix))
        .collect();
    match matches.as_slice() {
        [] => bail!("no {noun} matches `{prefix}`"),
        [one] => Ok(one),
        many => bail!(
            "`{prefix}` matches {} {noun}s: {}",
            many.len(),
            many.join(", ")
        ),
    }
}

pub fn resolve_source_prefix<'a>(manifest: &'a Manifest, prefix: &str) -> Result<&'a str> {
    unique_prefix(
        manifest.sources.keys().map(String::as_str).collect(),
        prefix,
        "source",
    )
}

pub fn resolve_topic_prefix(deck: &Deck, prefix: &str) -> Result<String> {
    let ids: Vec<String> = deck.topic_ids().into_iter().flatten().collect();
    unique_prefix(ids.iter().map(String::as_str).collect(), prefix, "topic").map(str::to_string)
}

fn member_files(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(workspace::member_dir(root)) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "md")
                && path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        !workspace::is_sidecar_name(name)
                            && !workspace::is_conventional_non_deck(name)
                            && !workspace::is_conflict_name(name)
                    })
        })
        .collect();
    files.sort();
    files
}

pub fn resolve_deck(arg: &str, cwd: &Path, decks_dir: Option<&Path>) -> Result<PathBuf> {
    let direct = cwd.join(arg);
    if direct.is_file() {
        return Ok(direct);
    }
    let mut files = Vec::new();
    if let Some(root) = cwd.ancestors().find(|dir| workspace::has_manifest(dir)) {
        files.extend(member_files(root));
    }
    if let Some(dir) = decks_dir {
        for root in workspace::roots_under(dir) {
            files.extend(member_files(&root));
        }
    }
    let mut seen = workspace::SeenPaths::default();
    let mut matches: Vec<(String, PathBuf)> = Vec::new();
    for file in files {
        if !seen.first_visit(&file) {
            continue;
        }
        if let Ok(Some(id)) = parser::read_deck_identity(&file)
            && !arg.is_empty()
            && id.starts_with(arg)
        {
            matches.push((id, file));
        }
    }
    let exact: Vec<&(String, PathBuf)> = matches.iter().filter(|(id, _)| id == arg).collect();
    let chosen: Vec<&(String, PathBuf)> = if exact.is_empty() {
        matches.iter().collect()
    } else {
        exact
    };
    match chosen.as_slice() {
        [] => bail!("no deck matches `{arg}`"),
        [(_, path)] => Ok(path.clone()),
        many => {
            let listed: Vec<String> = many
                .iter()
                .map(|(id, path)| format!("{id} ({})", path.display()))
                .collect();
            bail!(
                "`{arg}` matches {} decks: {}",
                many.len(),
                listed.join(", ")
            )
        }
    }
}

fn parse_topic_arg(deck: &Deck, arg: &str) -> Result<Binding> {
    let (ranges, prefix) = match arg.rsplit_once('|') {
        Some((ranges, prefix)) => (Some(ranges), prefix),
        None => (None, arg),
    };
    let topic = resolve_topic_prefix(deck, prefix)?;
    let entry = ranges.map_or_else(|| topic.clone(), |ranges| format!("{ranges}|{topic}"));
    binding_of(&entry).map_err(|problem| {
        anyhow!("topic argument `{arg}` {problem}; write it as `[<ranges>|]<topic>`")
    })
}

fn parse_topic_args(deck: &Deck, args: &[String]) -> Result<Vec<Binding>> {
    let mut bindings: Vec<Binding> = Vec::with_capacity(args.len());
    for arg in args {
        let binding = parse_topic_arg(deck, arg)?;
        if bindings.iter().any(|seen| seen.topic == binding.topic) {
            bail!("`{}` is named twice; one entry per topic", binding.topic);
        }
        bindings.push(binding);
    }
    Ok(bindings)
}

fn render_range(start: u32, end: u32) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}-{end}")
    }
}

fn check_ranges(binding: &Binding, lines: usize, what: &str) -> Result<()> {
    for &(start, end) in &binding.ranges {
        if end as usize > lines {
            bail!(
                "range {} lies beyond line {lines} of {what}",
                render_range(start, end)
            );
        }
    }
    Ok(())
}

struct Placed {
    display: DisplayPath,
    bytes: Vec<u8>,
}

fn read_source(file: &Path, roots: &Roots, key: Option<&str>) -> Result<Placed> {
    if file.to_str().is_some_and(deck::is_url) {
        bail!(
            "`{}` is a URL; sources are local files, so save the page as a file and add that file",
            file.display()
        );
    }
    let canonical =
        std::fs::canonicalize(file).with_context(|| format!("cannot read `{}`", file.display()))?;
    let (key, dir) = match key {
        Some(key) => {
            let Some(dir) = roots.roots.get(key) else {
                bail!(
                    "no root `{key}` is saved; save its folder with `--root {key}=<dir>` or \
                     `alix deck source root set`"
                );
            };
            if !canonical.starts_with(dir) {
                bail!(
                    "`{}` resolves to `{}`, outside root `{key}` (`{}`)",
                    file.display(),
                    canonical.display(),
                    dir.display()
                );
            }
            (key, dir)
        }
        None => {
            let within: Vec<(&String, &PathBuf)> = roots
                .roots
                .iter()
                .filter(|(_, dir)| canonical.starts_with(dir))
                .collect();
            match within.as_slice() {
                [] => bail!(
                    "`{}` is under no saved root; save its folder with `--root <key>=<dir>` or \
                     `alix deck source root set`",
                    file.display()
                ),
                [(key, dir)] => (key.as_str(), *dir),
                many => {
                    let keys: Vec<&str> = many.iter().map(|(key, _)| key.as_str()).collect();
                    bail!(
                        "`{}` is under roots {}; choose one with `--key`",
                        file.display(),
                        keys.join(", ")
                    )
                }
            }
        }
    };
    let mut components = Vec::new();
    for component in canonical
        .strip_prefix(dir)
        .unwrap_or(&canonical)
        .components()
    {
        let Component::Normal(name) = component else {
            bail!("`{}` is not inside root `{key}`", file.display());
        };
        let Some(name) = name.to_str() else {
            bail!("`{}` has a name that is not UTF-8", file.display());
        };
        components.push(name);
    }
    let display: DisplayPath = format!("{key}:{}", components.join("/")).parse()?;
    let bytes =
        std::fs::read(&canonical).with_context(|| format!("cannot read `{}`", file.display()))?;
    check_text(&bytes, &file.display().to_string())?;
    Ok(Placed { display, bytes })
}

fn check_text(bytes: &[u8], what: &str) -> Result<()> {
    if bytes.is_empty() {
        bail!("`{what}`: the file is empty");
    }
    if std::str::from_utf8(bytes).is_err() {
        bail!("`{what}` is not UTF-8 text; the exam reads text");
    }
    Ok(())
}

fn parse_root_key(key: &str) -> Result<String> {
    locator::parse_key(key).map_err(|_| {
        anyhow!("`{key}` is not a root key: a lowercase letter, then lowercase letters, digits, `_` or `-`")
    })
}

pub fn add(req: AddRequest, exam: &ExamConfig) -> Result<AddReport> {
    let mut opened = open(req.deck)?;
    let bindings = if req.topics.is_empty() {
        if opened
            .deck
            .topics
            .iter()
            .any(|topic| topic.heading.is_some())
        {
            bail!(
                "`--topic` is required: the deck has `#` topics, so name the ones this source grounds"
            );
        }
        vec![Binding {
            topic: opened.deck_id.clone(),
            ranges: Vec::new(),
        }]
    } else {
        parse_topic_args(&opened.deck, &req.topics)?
    };
    let mut roots = opened.roots.clone();
    let mut roots_changed = false;
    let mut key = req.key.clone();
    if let Some((root_key, dir)) = &req.root {
        let root_key = parse_root_key(root_key)?;
        let canonical = canonical_folder(dir)?;
        match roots.roots.get(&root_key) {
            Some(saved) if *saved != canonical => bail!(
                "root `{root_key}` is already `{}`; change it with `alix deck source root set` first",
                saved.display()
            ),
            Some(_) => {}
            None => {
                roots.roots.insert(root_key.clone(), canonical);
                roots_changed = true;
            }
        }
        key = key.or(Some(root_key));
    }
    let mut manifest = opened.manifest.clone();
    let mut copies = std::mem::take(&mut opened.copies);
    let mut new_copies = Vec::with_capacity(req.files.len());
    let mut added = Vec::with_capacity(req.files.len());
    for file in &req.files {
        let placed = read_source(file, &roots, key.as_deref())?;
        let path = placed.display.to_string();
        if manifest.sources.values().any(|row| row.path == path) {
            bail!(
                "`{path}` is already a source; refreeze it with `alix deck source update` or \
                 `alix doctor --repair-sources`"
            );
        }
        let id = source_id(&placed.bytes);
        if let Some(twin) = manifest.sources.get(&id) {
            bail!(
                "`{path}` has the same bytes as `{}`; a deck keeps one copy of each file",
                twin.path
            );
        }
        let lines = line_count(&placed.bytes);
        for binding in &bindings {
            check_ranges(binding, lines, &format!("`{path}`"))?;
        }
        manifest.sources.insert(
            id.clone(),
            Row {
                path: path.clone(),
                bound_to: bindings.iter().map(Binding::render).collect(),
            },
        );
        new_copies.push((copy_name(&id, &placed.display), placed.bytes.clone()));
        copies.insert(id.clone(), placed.bytes);
        added.push((id, path));
    }
    let sources = DeckSources {
        dir: opened.dir.clone(),
        manifest,
        copies,
    };
    let over_limit = over_limit_of(&opened.deck, &sources, exam)?;
    let report = finish(
        &opened,
        sources.manifest,
        new_copies,
        roots_changed.then_some(roots),
    )?;
    Ok(AddReport {
        added,
        collected: report.collected,
        over_limit,
    })
}

pub fn remove(deck: &Path, sources: &[String]) -> Result<RemoveReport> {
    let opened = open(deck)?;
    let mut manifest = opened.manifest.clone();
    let mut removed = Vec::with_capacity(sources.len());
    for prefix in sources {
        let id = resolve_source_prefix(&opened.manifest, prefix)?;
        if let Some(row) = manifest.sources.remove(id) {
            removed.push((id.to_string(), row.path));
        }
    }
    let report = finish(&opened, manifest, Vec::new(), None)?;
    Ok(RemoveReport {
        removed,
        collected: report.collected,
    })
}

pub fn update(
    deck: &Path,
    source: &str,
    file: &Path,
    key: Option<&str>,
    keep_results: bool,
    exam: &ExamConfig,
) -> Result<UpdateReport> {
    let mut opened = open(deck)?;
    let keep = keep_results
        .then(|| open_keep_results(&opened))
        .transpose()?;
    let old_id = resolve_source_prefix(&opened.manifest, source)?.to_string();
    let mut manifest = opened.manifest.clone();
    let old_row = manifest
        .sources
        .remove(&old_id)
        .with_context(|| format!("no source matches `{source}`"))?;
    let placed = read_source(file, &opened.roots, key)?;
    let path = placed.display.to_string();
    if let Some((other, _)) = manifest.sources.iter().find(|(_, row)| row.path == path) {
        bail!(
            "`{path}` is already source `{other}`; refreeze that one with `alix deck source \
             update` or `alix doctor --repair-sources`"
        );
    }
    let new_id = source_id(&placed.bytes);
    if let Some(twin) = manifest.sources.get(&new_id) {
        bail!(
            "`{path}` has the same bytes as `{}`; a deck keeps one copy of each file",
            twin.path
        );
    }
    let lines = line_count(&placed.bytes);
    let mut kept_ranges = Vec::new();
    for entry in &old_row.bound_to {
        let binding = parse_binding(&old_id, entry)?;
        check_ranges(&binding, lines, "the new copy")?;
        if !binding.ranges.is_empty() {
            kept_ranges.push(entry.clone());
        }
    }
    manifest.sources.insert(
        new_id.clone(),
        Row {
            path: path.clone(),
            bound_to: old_row.bound_to,
        },
    );
    let mut copies = std::mem::take(&mut opened.copies);
    copies.remove(&old_id);
    copies.insert(new_id.clone(), placed.bytes.clone());
    let sources = DeckSources {
        dir: opened.dir.clone(),
        manifest,
        copies,
    };
    let over_limit = over_limit_of(&opened.deck, &sources, exam)?;
    let new_copies = vec![(copy_name(&new_id, &placed.display), placed.bytes)];
    let report = finish(&opened, sources.manifest.clone(), new_copies, None)?;
    if let Some(keep) = keep {
        keep_topic_results(&opened, &sources.manifest, keep)?;
    }
    Ok(UpdateReport {
        old_id,
        new_id,
        path,
        kept_ranges,
        collected: report.collected,
        over_limit,
    })
}

pub fn bind(deck: &Path, source: &str, topics: &[String], exam: &ExamConfig) -> Result<BindReport> {
    let mut opened = open(deck)?;
    let id = resolve_source_prefix(&opened.manifest, source)?.to_string();
    let bindings = parse_topic_args(&opened.deck, topics)?;
    let lines = opened.copies.get(&id).map_or(0, |bytes| line_count(bytes));
    let mut manifest = opened.manifest.clone();
    let row = manifest
        .sources
        .get_mut(&id)
        .with_context(|| format!("no source matches `{source}`"))?;
    for binding in &bindings {
        check_ranges(binding, lines, &format!("`{}`", row.path))?;
        let entry = binding.render();
        match row
            .bound_to
            .iter()
            .position(|existing| topic_of(existing) == binding.topic)
        {
            Some(index) => row.bound_to[index] = entry,
            None => row.bound_to.push(entry),
        }
    }
    let sources = DeckSources {
        dir: opened.dir.clone(),
        manifest,
        copies: std::mem::take(&mut opened.copies),
    };
    let over_limit = over_limit_of(&opened.deck, &sources, exam)?;
    finish(&opened, sources.manifest, Vec::new(), None)?;
    Ok(BindReport { over_limit })
}

pub fn unbind(deck: &Path, source: &str, topics: &[String]) -> Result<()> {
    let opened = open(deck)?;
    let id = resolve_source_prefix(&opened.manifest, source)?.to_string();
    let mut manifest = opened.manifest.clone();
    let row = manifest
        .sources
        .get_mut(&id)
        .with_context(|| format!("no source matches `{source}`"))?;
    for prefix in topics {
        let topic = resolve_topic_prefix(&opened.deck, prefix)?;
        let Some(index) = row
            .bound_to
            .iter()
            .position(|entry| topic_of(entry) == topic)
        else {
            bail!("`{}` is not bound to `{topic}`", row.path);
        };
        row.bound_to.remove(index);
    }
    if row.bound_to.is_empty() {
        bail!(
            "`{}` would be bound to no topic; drop the source with `alix deck source remove`",
            row.path
        );
    }
    finish(&opened, manifest, Vec::new(), None)?;
    Ok(())
}

pub fn root_set(deck: &Path, key: &str, dir: &Path) -> Result<RootChange> {
    let opened = open(deck)?;
    let key = parse_root_key(key)?;
    let new = canonical_folder(dir)?;
    let mut roots = opened.roots;
    let old = roots.roots.insert(key, new.clone());
    save_roots(&opened.dir, &roots)?;
    Ok(RootChange { old, new })
}

pub fn root_unset(deck: &Path, key: &str) -> Result<Option<PathBuf>> {
    let opened = open(deck)?;
    let mut roots = opened.roots;
    let old = roots.roots.remove(key);
    if old.is_some() {
        save_roots(&opened.dir, &roots)?;
    }
    Ok(old)
}

pub fn list(deck: &Path, exam: &ExamConfig) -> Result<Listing> {
    let Opened {
        deck,
        deck_id,
        dir,
        manifest,
        copies,
        roots,
        ..
    } = open(deck)?;
    let held = DeckSources {
        dir,
        manifest,
        copies,
    };
    let measured = topic_measures(&deck, &held, exam)?;
    let mut topics = Vec::new();
    for (index, topic) in deck.topics.iter().enumerate() {
        let Some(id) = topic.id(&deck_id) else {
            continue;
        };
        let bound: Vec<&String> = held
            .manifest
            .sources
            .iter()
            .filter(|(_, row)| row.bound_to.iter().any(|entry| topic_of(entry) == id))
            .map(|(source, _)| source)
            .collect();
        let label = match &topic.heading {
            Some(heading) => heading.clone(),
            None => {
                let holds_card = deck.cards.iter().any(|card| card.topic == index);
                if !holds_card && bound.is_empty() {
                    continue;
                }
                deck.display_name()
            }
        };
        let source_bytes = bound
            .iter()
            .filter_map(|source| held.copies.get(*source))
            .map(|bytes| bytes.len() as u64)
            .sum();
        let measures = measured
            .iter()
            .find(|topic| topic.topic_id == id)
            .map(|topic| topic.measures.clone())
            .unwrap_or_default();
        topics.push(TopicLine {
            id,
            label,
            source_count: bound.len(),
            source_bytes,
            measures,
        });
    }
    let mut sources = Vec::with_capacity(held.manifest.sources.len());
    for (id, row) in &held.manifest.sources {
        let display: DisplayPath = row.path.parse()?;
        let issue = root_issue(&roots.roots, &display.key);
        let drift = match (&issue, roots.roots.get(&display.key)) {
            (None, Some(dir)) => live_state(id, &display, dir).0,
            _ => Drift::Unchecked,
        };
        sources.push((
            display,
            SourceLine {
                id: id.clone(),
                bytes: held.copies.get(id).map_or(0, |bytes| bytes.len() as u64),
                path: row.path.clone(),
                bound: row.bound_to.clone(),
                drift,
                root_issue: issue,
            },
        ));
    }
    sources.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(Listing {
        topics,
        sources: sources.into_iter().map(|(_, line)| line).collect(),
    })
}

pub(super) fn live_state(id: &str, display: &DisplayPath, dir: &Path) -> (Drift, Option<Vec<u8>>) {
    let live = display
        .path
        .split('/')
        .fold(dir.to_path_buf(), |live, part| live.join(part));
    match std::fs::symlink_metadata(&live) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (Drift::Missing, None);
        }
        Err(_) => return (Drift::Unreadable, None),
        Ok(metadata) if metadata.file_type().is_symlink() => return (Drift::Linked, None),
        Ok(_) => {}
    }
    match std::fs::canonicalize(&live) {
        Ok(canonical) if canonical.starts_with(dir) => {}
        Ok(_) => return (Drift::Linked, None),
        Err(_) => return (Drift::Unreadable, None),
    }
    match std::fs::read(&live) {
        Ok(bytes) if source_id(&bytes) == id => (Drift::Same, None),
        Ok(bytes) => (Drift::Changed, Some(bytes)),
        Err(_) => (Drift::Unreadable, None),
    }
}

pub fn repair(deck: &Path, keep_results: bool, exam: &ExamConfig) -> Result<RepairReport> {
    let mut opened = open(deck)?;
    let keep = keep_results
        .then(|| open_keep_results(&opened))
        .transpose()?;
    let mut report = RepairReport::default();
    let mut rows: Vec<(String, Row)> = Vec::with_capacity(opened.manifest.sources.len());
    let mut new_copies = Vec::new();
    let mut copies = std::mem::take(&mut opened.copies);
    let mut ordered: Vec<(&String, &Row)> = opened.manifest.sources.iter().collect();
    ordered.sort_by(|a, b| a.1.path.cmp(&b.1.path));
    for (id, row) in ordered {
        let display: DisplayPath = row.path.parse()?;
        let key = display.key.as_str();
        let issue = root_issue(&opened.roots.roots, key);
        let state = match (&issue, opened.roots.roots.get(key)) {
            (None, Some(dir)) => live_state(id, &display, dir),
            _ => (Drift::Unchecked, None),
        };
        let kept_id = match state {
            (Drift::Changed, Some(bytes)) => {
                check_text(&bytes, &row.path)?;
                let lines = line_count(&bytes);
                for entry in &row.bound_to {
                    let binding = parse_binding(id, entry)?;
                    check_ranges(&binding, lines, &format!("the live `{}`", row.path))?;
                    if !binding.ranges.is_empty() {
                        report.kept_ranges.push((row.path.clone(), entry.clone()));
                    }
                }
                let new_id = source_id(&bytes);
                new_copies.push((copy_name(&new_id, &display), bytes.clone()));
                copies.remove(id);
                copies.insert(new_id.clone(), bytes);
                report.refrozen.push((new_id.clone(), row.path.clone()));
                new_id
            }
            (Drift::Same, _) => id.clone(),
            (Drift::Missing, _) => {
                report.missing.push(row.path.clone());
                id.clone()
            }
            (other, _) => {
                let reason = match other {
                    Drift::Linked => {
                        format!("the live file is a symbolic link or leaves root `{key}`")
                    }
                    Drift::Unreadable => "the live file cannot be read".to_string(),
                    _ => format!(
                        "`{key}`: {} {}",
                        issue.as_deref().unwrap_or_default(),
                        root_fix(&opened.deck_id, key)
                    ),
                };
                report.skipped.push((row.path.clone(), reason));
                id.clone()
            }
        };
        rows.push((kept_id, row.clone()));
    }
    let mut holders: BTreeMap<&str, &str> = BTreeMap::new();
    for (id, row) in &rows {
        if let Some(other) = holders.insert(id, &row.path) {
            bail!(
                "`{other}` and `{}` would hold the same bytes; a deck keeps one copy of each \
                 file, so nothing was refrozen",
                row.path
            );
        }
    }
    if report.refrozen.is_empty() && stray_copies(&opened.dir)?.is_empty() {
        return Ok(report);
    }
    let sources = DeckSources {
        dir: opened.dir.clone(),
        manifest: Manifest {
            version: MANIFEST_VERSION,
            sources: rows.into_iter().collect(),
        },
        copies,
    };
    report.over_limit = over_limit_of(&opened.deck, &sources, exam)?;
    report.collected = finish(&opened, sources.manifest.clone(), new_copies, None)?.collected;
    if let Some(keep) = keep {
        keep_topic_results(&opened, &sources.manifest, keep)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        exam_status::{self, TopicStatus},
        sources::{
            CommitError,
            testkit::{DECK, TOPIC_A, TOPIC_B, Workspace},
        },
        state,
    };

    const A: &[u8] = b"one\ntwo\nthree\nfour\nfive\n";
    const C: &[u8] = b"1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n16\n17\n18\n19\n20\n21\n";
    const EDITED: &[u8] = b"one\ntwo\nthree\nfour\nfive, edited\n";
    const NOW: u64 = 1_000_000;

    const EXAMINED_DECK: &str = "---\nid: \"deck-fixture\"\n---\n# Alpha\n<!-- id: topic-alpha \
                                 -->\n\n## q1\na1\n<!-- id: card-q1 -->\n\n# Beta\n<!-- id: \
                                 topic-beta -->\n\n## q3\na3\n<!-- id: card-q3 -->\n\n## \
                                 q4\na4\n<!-- id: card-q4 -->\n";

    fn first(_: &Workspace) -> String {
        source_id(A)
    }

    fn exam() -> ExamConfig {
        ExamConfig::default()
    }

    fn over_limit_lines(listing: &Listing) -> Vec<String> {
        listing
            .topics
            .iter()
            .flat_map(|line| {
                line.measures
                    .iter()
                    .filter(|measure| measure.exceeds())
                    .map(|&measure| {
                        PreflightError {
                            topic: line.label.clone(),
                            measure,
                        }
                        .to_string()
                    })
            })
            .collect()
    }

    fn lines(over_limit: &[PreflightError]) -> Vec<String> {
        over_limit.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn source_commands_warn_but_never_refuse_on_an_oversized_topic_and_list_shows_the_same_number()
    {
        let workspace = Workspace::new();
        let big = format!("{}\n", "x".repeat(329_999));
        workspace.write_live("big.txt", big.as_bytes());
        let id = source_id(big.as_bytes());

        let added = workspace
            .add(&["big.txt"], &[TOPIC_A])
            .unwrap_or_else(|error| panic!("add: {error:#}"));
        let listing = list(&workspace.deck(), &exam()).unwrap();
        let alpha = listing
            .topics
            .iter()
            .find(|line| line.id == TOPIC_A)
            .unwrap();
        assert_eq!(
            vec![BackendKind::Gemini, BackendKind::Copilot],
            alpha
                .measures
                .iter()
                .filter(|measure| measure.exceeds())
                .map(|measure| measure.backend)
                .collect::<Vec<_>>(),
            "list: 330 KB exceeds the 128,000-token backends only: {:?}",
            alpha.measures
        );
        let from_list = over_limit_lines(&listing);
        assert_eq!(
            from_list,
            lines(&added.over_limit),
            "add: warns with list's numbers and never refuses"
        );
        for line in &from_list {
            assert!(
                line.starts_with("topic `Alpha` exceeds ")
                    && line.ends_with(" of 128000 estimated tokens"),
                "add: the warning names the topic, the backend, both sizes and the unit: {line}"
            );
        }

        let deck = Deck::load(workspace.deck()).unwrap();
        let sources = load_verified(workspace.root(), DECK).unwrap().unwrap();
        let topics = exam_status::examined_topics(&deck);
        for kind in BackendKind::ALL {
            let ask = crate::config::AskConfig {
                backend: kind,
                ..Default::default()
            };
            let expected = from_list
                .iter()
                .find(|line| line.contains(&format!(" exceeds {}'s ", kind.name())))
                .cloned()
                .map_or(Ok(()), Err);
            assert_eq!(
                expected,
                crate::exam::preflight_topics(&topics, &sources, &exam(), &ask)
                    .map_err(|error| format!("{error:#}")),
                "preflight for {kind:?} says what add said"
            );
        }
        assert_eq!(
            from_list,
            crate::sources::deck_findings(workspace.root(), &deck, false, &exam()).warnings,
            "doctor: the same lines as warnings"
        );

        let bound = bind(&workspace.deck(), &id, &strings(&[TOPIC_B]), &exam())
            .unwrap_or_else(|error| panic!("bind: {error:#}"));
        let listing = list(&workspace.deck(), &exam()).unwrap();
        assert_eq!(
            over_limit_lines(&listing),
            lines(&bound.over_limit),
            "bind: Beta's lines join Alpha's, with list's numbers"
        );
        assert_eq!(
            (4, 2),
            (
                bound.over_limit.len(),
                bound
                    .over_limit
                    .iter()
                    .filter(|line| line.topic == "Beta")
                    .count()
            ),
            "bind: two backends per bound topic"
        );
        let alpha_before = listing.topics[0].measures.to_vec();

        let bigger = format!("{}\n", "y".repeat(400_000));
        workspace.write_live("big.txt", bigger.as_bytes());
        let updated = update(
            &workspace.deck(),
            &id,
            &workspace.live().join("big.txt"),
            None,
            false,
            &exam(),
        )
        .unwrap_or_else(|error| panic!("update: {error:#}"));
        let listing = list(&workspace.deck(), &exam()).unwrap();
        assert_eq!(
            over_limit_lines(&listing),
            lines(&updated.over_limit),
            "update: warns with list's numbers and never refuses"
        );
        assert!(
            listing.topics[0]
                .measures
                .iter()
                .zip(&alpha_before)
                .all(|(after, before)| after.size > before.size),
            "update: every backend's reading grew with the file"
        );

        workspace.write_live("big.txt", format!("{}\n", "z".repeat(350_000)).as_bytes());
        let repaired = repair(&workspace.deck(), false, &exam())
            .unwrap_or_else(|error| panic!("repair: {error:#}"));
        let listing = list(&workspace.deck(), &exam()).unwrap();
        assert_eq!(
            (1, over_limit_lines(&listing)),
            (repaired.refrozen.len(), lines(&repaired.over_limit)),
            "repair: refreezes and warns with list's numbers"
        );
    }

    fn base() -> Workspace {
        let workspace = Workspace::new();
        workspace.write_live("a.rs", A);
        workspace.write_live("c.rs", C);
        workspace.add(&["a.rs"], &[TOPIC_A]).unwrap();
        workspace
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    type Row = (
        &'static str,
        fn(&Workspace),
        fn(&Workspace) -> anyhow::Result<()>,
        &'static [&'static str],
    );

    fn add_c(workspace: &Workspace, topic: &str) -> anyhow::Result<()> {
        workspace.add(&["c.rs"], &[topic]).map(drop)
    }

    fn rows() -> Vec<Row> {
        let mut rows: Vec<Row> = vec![
            (
                "twin",
                |w| drop(w.write_live("b.rs", A)),
                |w| w.add(&["b.rs"], &[TOPIC_A]).map(drop),
                &["sdk:a.rs", "sdk:b.rs"],
            ),
            (
                "non-utf8",
                |w| drop(w.write_live("bad.txt", &[b'o', 0xff, b'\n'])),
                |w| w.add(&["bad.txt"], &[TOPIC_A]).map(drop),
                &["not UTF-8", "bad.txt"],
            ),
            (
                "bad range reversed",
                |_| {},
                |w| add_c(w, "80-10|topic-alpha"),
                &["range"],
            ),
            (
                "bad range single as range",
                |_| {},
                |w| add_c(w, "10-10|topic-alpha"),
                &["range"],
            ),
            (
                "bad range out of order",
                |_| {},
                |w| add_c(w, "5,3|topic-alpha"),
                &["range"],
            ),
            (
                "bad range overlapping",
                |_| {},
                |w| add_c(w, "1-9,5-20|topic-alpha"),
                &["range"],
            ),
            (
                "bad range zero",
                |_| {},
                |w| add_c(w, "0|topic-alpha"),
                &["range"],
            ),
            (
                "bad range beyond the file",
                |_| {},
                |w| add_c(w, "1-99999|topic-alpha"),
                &["range", "beyond line 21"],
            ),
            (
                "bad range two bars",
                |_| {},
                |w| add_c(w, "1|2|topic-alpha"),
                &["range"],
            ),
            (
                "update out of bounds",
                |w| {
                    bind(&w.deck(), &first(w), &strings(&["2-5|topic-beta"]), &exam()).unwrap();
                    w.write_live("a.rs", b"one\ntwo\n");
                },
                |w| {
                    update(
                        &w.deck(),
                        &first(w),
                        &w.live().join("a.rs"),
                        None,
                        false,
                        &exam(),
                    )
                    .map(drop)
                },
                &["range 2-5 lies beyond line 2 of the new copy"],
            ),
            (
                "empty file on add",
                |w| drop(w.write_live("empty.txt", b"")),
                |w| w.add(&["empty.txt"], &[TOPIC_A]).map(drop),
                &["the file is empty", "empty.txt"],
            ),
            (
                "empty file on update",
                |w| drop(w.write_live("a.rs", b"")),
                |w| {
                    update(
                        &w.deck(),
                        &first(w),
                        &w.live().join("a.rs"),
                        None,
                        false,
                        &exam(),
                    )
                    .map(drop)
                },
                &["the file is empty"],
            ),
            (
                "duplicate display path",
                |w| drop(w.write_live("a.rs", b"changed\n")),
                |w| w.add(&["a.rs"], &[TOPIC_A]).map(drop),
                &["sdk:a.rs", "--repair-sources"],
            ),
            (
                "no root",
                |w| fs::write(w.root().join("outside.rs"), b"x\n").unwrap(),
                |w| {
                    w.add_bare(&[w.root().join("outside.rs")], &[TOPIC_A])
                        .map(drop)
                },
                &["no saved root", "--root"],
            ),
            (
                "root overwrite",
                |w| fs::create_dir_all(w.root().join("other")).unwrap(),
                |w| {
                    add(
                        AddRequest {
                            deck: &w.deck(),
                            files: vec![w.live().join("c.rs")],
                            root: Some(("sdk".to_string(), w.root().join("other"))),
                            key: None,
                            topics: strings(&[TOPIC_A]),
                        },
                        &exam(),
                    )
                    .map(drop)
                },
                &["root set"],
            ),
            (
                "root set to a missing folder",
                |_| {},
                |w| root_set(&w.deck(), "dl", &w.root().join("gone")).map(drop),
                &["gone"],
            ),
            (
                "two roots",
                |w| {
                    w.write_live("sub/x.rs", b"x\n");
                    root_set(&w.deck(), "sub", &w.live().join("sub")).unwrap();
                },
                |w| {
                    w.add_bare(&[w.live().join("sub/x.rs")], &[TOPIC_A])
                        .map(drop)
                },
                &["sdk, sub", "--key"],
            ),
            (
                "unknown key",
                |_| {},
                |w| {
                    add(
                        AddRequest {
                            deck: &w.deck(),
                            files: vec![w.live().join("c.rs")],
                            root: None,
                            key: Some("dl".to_string()),
                            topics: strings(&[TOPIC_A]),
                        },
                        &exam(),
                    )
                    .map(drop)
                },
                &["no root `dl`"],
            ),
            (
                "unknown topic",
                |_| {},
                |w| add_c(w, "topic-zzz"),
                &["no topic", "topic-zzz"],
            ),
            (
                "ambiguous topic prefix",
                |_| {},
                |w| add_c(w, "topic-alp"),
                &["matches 2"],
            ),
            (
                "topic required",
                |_| {},
                |w| w.add_bare(&[w.live().join("c.rs")], &[]).map(drop),
                &["--topic"],
            ),
            (
                "topic named twice",
                |_| {},
                |w| w.add(&["c.rs"], &[TOPIC_A, "1|topic-alpha"]).map(drop),
                &["named twice"],
            ),
            (
                "duplicated topic id in the deck",
                |w| {
                    let text = fs::read_to_string(w.deck()).unwrap();
                    fs::write(w.deck(), text.replace("topic-beta", "topic-alpha")).unwrap();
                },
                |w| add_c(w, TOPIC_A),
                &["topic id `topic-alpha` is also at", "alix deck init"],
            ),
            (
                "duplicated topic id across decks",
                |w| {
                    fs::write(
                        w.root().join("decks").join("other.md"),
                        "---\nid: \"deck-other\"\n---\n# X\n<!-- id: topic-beta -->\n\n## q\na\n",
                    )
                    .unwrap();
                },
                |w| bind(&w.deck(), &first(w), &strings(&[TOPIC_B]), &exam()).map(drop),
                &["topic id `topic-beta` is also at", "other.md"],
            ),
            (
                "duplicated topic id in the deck, on bind",
                |w| {
                    let text = fs::read_to_string(w.deck()).unwrap();
                    fs::write(w.deck(), text.replace("topic-beta", "topic-alpha")).unwrap();
                },
                |w| bind(&w.deck(), &first(w), &strings(&[TOPIC_A]), &exam()).map(drop),
                &["topic id `topic-alpha` is also at", "d.md:"],
            ),
            (
                "duplicated topic id across decks, on add",
                |w| {
                    fs::write(
                        w.root().join("decks").join("other.md"),
                        "---\nid: \"deck-other\"\n---\n# X\n<!-- id: topic-beta -->\n\n## q\na\n",
                    )
                    .unwrap();
                },
                |w| add_c(w, TOPIC_B),
                &["topic id `topic-beta` is also at", "other.md:"],
            ),
            (
                "last unbind",
                |_| {},
                |w| unbind(&w.deck(), &first(w), &strings(&[TOPIC_A])),
                &["alix deck source remove"],
            ),
            (
                "unbind of an unbound topic",
                |_| {},
                |w| unbind(&w.deck(), &first(w), &strings(&[TOPIC_B])),
                &["is not bound to `topic-beta`"],
            ),
            (
                "loose deck",
                |w| {
                    fs::write(
                        w.root().join("loose.md"),
                        "---\nid: \"deck-loose\"\n---\n## q\na\n",
                    )
                    .unwrap()
                },
                |w| {
                    add(
                        AddRequest {
                            deck: &w.root().join("loose.md"),
                            files: vec![w.live().join("c.rs")],
                            root: Some(("sdk".to_string(), w.live())),
                            key: None,
                            topics: Vec::new(),
                        },
                        &exam(),
                    )
                    .map(drop)
                },
                &["not a workspace member"],
            ),
            (
                "unknown source prefix",
                |_| {},
                |w| remove(&w.deck(), &strings(&["source-9"])).map(drop),
                &["no source matches `source-9`"],
            ),
            (
                "empty bound-to",
                |w| w.rewrite_manifest("bound-to = [\"topic-alpha\"]", "bound-to = []"),
                |w| bind(&w.deck(), &first(w), &strings(&[TOPIC_B]), &exam()).map(drop),
                &["bound-to"],
            ),
            (
                "missing topic binding",
                |w| w.rewrite_manifest("\"topic-alpha\"", "\"topic-gone\""),
                |w| bind(&w.deck(), &first(w), &strings(&[TOPIC_B]), &exam()).map(drop),
                &["names no topic", "topic-gone"],
            ),
            (
                "version",
                |w| w.rewrite_manifest("version = 1", "version = 2"),
                |w| bind(&w.deck(), &first(w), &strings(&[TOPIC_B]), &exam()).map(drop),
                &["version 2"],
            ),
            (
                "digest",
                |w| {
                    let copy = w.dir().join(format!("{}.rs", first(w)));
                    let mut bytes = fs::read(&copy).unwrap();
                    bytes[0] ^= 1;
                    fs::write(copy, bytes).unwrap();
                },
                |w| bind(&w.deck(), &first(w), &strings(&[TOPIC_B]), &exam()).map(drop),
                &["does not match its id"],
            ),
            (
                "url",
                |_| {},
                |w| {
                    w.add_bare(&[PathBuf::from("https://example.com/page")], &[TOPIC_A])
                        .map(drop)
                },
                &["is a URL", "save the page"],
            ),
        ];
        rows.extend(platform_rows());
        rows
    }

    #[cfg(unix)]
    fn platform_rows() -> Vec<Row> {
        vec![
            (
                "containment",
                |w| {
                    fs::write(w.root().join("outside.rs"), b"outside\n").unwrap();
                    std::os::unix::fs::symlink(
                        w.root().join("outside.rs"),
                        w.live().join("link.rs"),
                    )
                    .unwrap();
                },
                |w| w.add(&["link.rs"], &[TOPIC_A]).map(drop),
                &["outside root `sdk`"],
            ),
            (
                "display path with a colon",
                |w| drop(w.write_live("a:b.rs", b"colon\n")),
                |w| w.add(&["a:b.rs"], &[TOPIC_A]).map(drop),
                &["contains a colon"],
            ),
        ]
    }

    #[cfg(not(unix))]
    fn platform_rows() -> Vec<Row> {
        Vec::new()
    }

    #[test]
    fn every_source_command_refusal_names_its_cause_and_leaves_state_untouched() {
        for (label, setup, command, must_contain) in rows() {
            let workspace = base();
            setup(&workspace);
            let before = workspace.snapshot();
            let error = match command(&workspace) {
                Ok(()) => panic!("{label}: the command succeeded"),
                Err(error) => format!("{error:#}"),
            };
            for words in must_contain {
                assert!(
                    error.contains(words),
                    "{label}: the error does not name {words:?}: {error}"
                );
            }
            assert_eq!(
                before,
                workspace.snapshot(),
                "{label}: the refusal changed the sources folder"
            );
        }
    }

    #[test]
    fn update_keeps_ranges_and_prints_them() {
        let workspace = base();
        let id = source_id(A);
        bind(
            &workspace.deck(),
            &id,
            &strings(&["2-4|topic-alpha", TOPIC_B]),
            &exam(),
        )
        .unwrap();
        let longer = [A, b"six\nseven\n"].concat();
        workspace.write_live("a.rs", &longer);

        let report = update(
            &workspace.deck(),
            "source-",
            &workspace.live().join("a.rs"),
            None,
            false,
            &exam(),
        )
        .unwrap_or_else(|error| panic!("update: {error:#}"));

        let new_id = source_id(&longer);
        assert_eq!(
            (id.as_str(), new_id.as_str(), "sdk:a.rs"),
            (
                report.old_id.as_str(),
                report.new_id.as_str(),
                report.path.as_str()
            ),
            "update: ids and display path"
        );
        assert_eq!(
            strings(&["2-4|topic-alpha"]),
            report.kept_ranges,
            "update: the ranged entry is listed for review"
        );
        let sources = load_verified(workspace.root(), DECK).unwrap().unwrap();
        assert_eq!(
            vec![new_id.clone()],
            sources.manifest.sources.keys().cloned().collect::<Vec<_>>(),
            "update: one row under the new id"
        );
        assert_eq!(
            strings(&["2-4|topic-alpha", TOPIC_B]),
            sources.manifest.sources[&new_id].bound_to,
            "update: every entry kept byte for byte"
        );
        assert_eq!(
            vec![format!("{id}.rs")],
            report.collected,
            "update: the old copy is collected"
        );
    }

    #[test]
    fn bind_replaces_an_existing_entry_remove_collects_and_list_reports_every_column() {
        let workspace = base();
        let id = source_id(A);
        bind(
            &workspace.deck(),
            "source",
            &strings(&["1-2|topic-alpha", TOPIC_B]),
            &exam(),
        )
        .unwrap();
        let sources = load_verified(workspace.root(), DECK).unwrap().unwrap();
        assert_eq!(
            strings(&["1-2|topic-alpha", TOPIC_B]),
            sources.manifest.sources[&id].bound_to,
            "bind: the alpha entry is replaced in place, beta appended"
        );

        workspace.add(&["c.rs"], &["topic-be"]).unwrap();
        workspace.write_live("c.rs", b"drifted\n");
        let listing = list(&workspace.deck(), &exam()).unwrap();
        let rows: Vec<(&str, u64, &str, Drift)> = listing
            .sources
            .iter()
            .map(|line| (line.id.as_str(), line.bytes, line.path.as_str(), line.drift))
            .collect();
        let c_id = source_id(C);
        assert_eq!(
            vec![
                (id.as_str(), A.len() as u64, "sdk:a.rs", Drift::Same),
                (c_id.as_str(), C.len() as u64, "sdk:c.rs", Drift::Changed),
            ],
            rows,
            "list: sources by display path with drift"
        );
        let topics: Vec<(&str, &str, usize, u64)> = listing
            .topics
            .iter()
            .map(|line| {
                (
                    line.id.as_str(),
                    line.label.as_str(),
                    line.source_count,
                    line.source_bytes,
                )
            })
            .collect();
        assert_eq!(
            vec![
                (TOPIC_A, "Alpha", 1, A.len() as u64),
                ("topic-alpine", "Alpine", 0, 0),
                (TOPIC_B, "Beta", 2, (A.len() + C.len()) as u64),
            ],
            topics,
            "list: the empty leading topic is omitted"
        );

        root_unset(&workspace.deck(), "sdk").unwrap();
        assert!(
            list(&workspace.deck(), &exam())
                .unwrap()
                .sources
                .iter()
                .all(|line| line.drift == Drift::Unchecked),
            "list: a key without a root is unchecked"
        );

        let report = remove(&workspace.deck(), &strings(&[&c_id[..12]])).unwrap();
        assert_eq!(vec![(c_id.clone(), "sdk:c.rs".to_string())], report.removed);
        assert_eq!(
            vec![format!("{c_id}.rs")],
            report.collected,
            "remove: the copy goes"
        );
        assert!(
            !workspace.dir().join(format!("{c_id}.rs")).exists(),
            "remove: the copy is gone from disk"
        );
    }

    #[test]
    fn a_deck_without_topics_binds_to_its_deck_id_and_root_set_reports_the_change() {
        let workspace = Workspace::new();
        fs::write(
            workspace.deck(),
            "---\nid: \"deck-fixture\"\n---\n## q\na\n",
        )
        .unwrap();
        workspace.write_live("a.rs", A);
        workspace.add(&["a.rs"], &[]).unwrap();
        let sources = load_verified(workspace.root(), DECK).unwrap().unwrap();
        assert_eq!(
            strings(&[DECK]),
            sources.manifest.sources[&source_id(A)].bound_to
        );
        let listing = list(&workspace.deck(), &exam()).unwrap();
        assert_eq!(1, listing.topics.len(), "the leading topic holds a card");
        assert_eq!(
            "d", listing.topics[0].label,
            "the leading topic is labelled by the deck name"
        );

        fs::create_dir_all(workspace.root().join("moved")).unwrap();
        let change = root_set(&workspace.deck(), "sdk", &workspace.root().join("moved")).unwrap();
        assert_eq!(
            RootChange {
                old: Some(fs::canonicalize(workspace.live()).unwrap()),
                new: fs::canonicalize(workspace.root().join("moved")).unwrap(),
            },
            change
        );
        assert_eq!(
            Some(fs::canonicalize(workspace.root().join("moved")).unwrap()),
            root_unset(&workspace.deck(), "sdk").unwrap()
        );
        assert_eq!(
            None,
            root_unset(&workspace.deck(), "sdk").unwrap(),
            "already unset"
        );
    }

    #[test]
    fn a_stale_saved_root_does_not_block_saving_another_key() {
        let workspace = base();
        let gone = workspace.root().join("gone");
        fs::create_dir_all(&gone).unwrap();
        root_set(&workspace.deck(), "old", &gone).unwrap();
        let saved = fs::canonicalize(&gone).unwrap();
        fs::remove_dir(&gone).unwrap();
        let other = workspace.root().join("other");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("o.txt"), b"other\n").unwrap();

        root_set(&workspace.deck(), "dl", &other)
            .unwrap_or_else(|error| panic!("root set beside a stale root: {error:#}"));
        add(
            AddRequest {
                deck: &workspace.deck(),
                files: vec![other.join("o.txt")],
                root: Some(("web".to_string(), other.clone())),
                key: Some("web".to_string()),
                topics: strings(&[TOPIC_A]),
            },
            &exam(),
        )
        .unwrap_or_else(|error| panic!("add --root beside a stale root: {error:#}"));
        let roots = Roots::load(&workspace.dir()).unwrap().unwrap();
        assert_eq!(
            Some(&saved),
            roots.roots.get("old"),
            "the stale root is kept as saved"
        );
    }

    #[test]
    fn list_reports_an_unreadable_live_file_as_drift_instead_of_failing() {
        let workspace = base();
        fs::remove_file(workspace.live().join("a.rs")).unwrap();
        fs::create_dir(workspace.live().join("a.rs")).unwrap();
        let listing =
            list(&workspace.deck(), &exam()).unwrap_or_else(|error| panic!("list: {error:#}"));
        assert_eq!(
            vec![Drift::Unreadable],
            listing
                .sources
                .iter()
                .map(|line| line.drift)
                .collect::<Vec<_>>(),
            "a folder where the live file was is unreadable, per source"
        );
    }

    #[cfg(unix)]
    #[test]
    fn list_never_follows_a_live_symlink_out_of_the_root() {
        let workspace = base();
        let outside = workspace.root().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("a.rs"), A).unwrap();
        fs::remove_file(workspace.live().join("a.rs")).unwrap();
        std::os::unix::fs::symlink(outside.join("a.rs"), workspace.live().join("a.rs")).unwrap();
        let drift = |workspace: &Workspace| -> Vec<Drift> {
            list(&workspace.deck(), &exam())
                .unwrap()
                .sources
                .iter()
                .map(|line| line.drift)
                .collect()
        };
        assert_eq!(vec![Drift::Linked], drift(&workspace), "a linked live file");

        let nested = base();
        let sub = nested.root().join("away");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("a.rs"), A).unwrap();
        fs::rename(nested.live(), nested.root().join("live-real")).unwrap();
        std::os::unix::fs::symlink(&sub, nested.live()).unwrap();
        assert_eq!(
            vec![Drift::Linked],
            drift(&nested),
            "a live path through a linked folder leaving the saved root"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_deck_folder_is_refused_and_nothing_behind_the_link_is_collected() {
        let workspace = base();
        let stray = format!("{}.txt", source_id(b"stray\n"));
        fs::write(workspace.dir().join(&stray), b"stray\n").unwrap();
        let real = workspace.root().join("real");
        fs::rename(workspace.dir(), &real).unwrap();
        std::os::unix::fs::symlink(&real, workspace.dir()).unwrap();

        let error = bind(
            &workspace.deck(),
            &source_id(A),
            &strings(&[TOPIC_B]),
            &exam(),
        )
        .unwrap_err();

        assert!(
            format!("{error:#}").contains("is a symbolic link"),
            "the linked folder is named: {error:#}"
        );
        assert!(
            real.join(&stray).is_file(),
            "the stray behind the link survives"
        );
        assert!(
            fs::read_to_string(real.join(crate::sources::MANIFEST))
                .unwrap()
                .contains("topic-alpha\"]"),
            "the manifest behind the link is unchanged"
        );
    }

    #[test]
    fn repair_refreezes_drifted_sources_and_refuses_a_convergence_naming_both_rows() {
        let workspace = base();
        workspace.add(&["c.rs"], &[TOPIC_B]).unwrap();
        let refusals: [(&str, &[u8], &[u8]); 2] = [
            ("two drifted files converge", b"same\n", b"same\n"),
            ("one becomes another row's bytes", C, C),
        ];
        for (label, a_bytes, c_bytes) in refusals {
            workspace.write_live("a.rs", a_bytes);
            workspace.write_live("c.rs", c_bytes);
            let before = workspace.snapshot();
            let error = format!(
                "{:#}",
                repair(&workspace.deck(), false, &exam())
                    .expect_err("a convergence refuses the repair")
            );
            for path in ["`sdk:a.rs`", "`sdk:c.rs`"] {
                assert!(error.contains(path), "{label}: names {path}: {error}");
            }
            assert_eq!(
                before,
                workspace.snapshot(),
                "{label}: the refusal leaves the folder"
            );
        }

        let edited = b"one\ntwo\nthree\nfour\nfive, edited\n";
        workspace.write_live("a.rs", edited);
        let report = repair(&workspace.deck(), false, &exam())
            .unwrap_or_else(|error| panic!("repair: {error:#}"));
        assert_eq!(
            vec![(source_id(edited), "sdk:a.rs".to_string())],
            report.refrozen,
            "repair: only the drifted row is refrozen"
        );
        assert_eq!(
            vec![format!("{}.rs", source_id(A))],
            report.collected,
            "repair: the old copy is collected"
        );
        let listed: Vec<(String, Drift)> = list(&workspace.deck(), &exam())
            .unwrap()
            .sources
            .into_iter()
            .map(|line| (line.id, line.drift))
            .collect();
        assert_eq!(
            vec![
                (source_id(edited), Drift::Same),
                (source_id(C), Drift::Same)
            ],
            listed,
            "list: the refrozen row under its new id"
        );
        assert_eq!(
            RepairReport::default(),
            repair(&workspace.deck(), false, &exam()).unwrap(),
            "a second repair finds nothing to do"
        );
    }

    #[test]
    fn repair_keeps_missing_and_unchecked_rows_and_refuses_a_range_beyond_the_new_copy() {
        let workspace = base();
        workspace.add(&["c.rs"], &[TOPIC_B]).unwrap();
        bind(
            &workspace.deck(),
            &source_id(C),
            &strings(&["20-21|topic-beta"]),
            &exam(),
        )
        .unwrap();
        workspace.write_live("c.rs", b"1\n2\n");
        let before = workspace.snapshot();
        let error = format!(
            "{:#}",
            repair(&workspace.deck(), false, &exam()).unwrap_err()
        );
        assert!(
            error.contains("range 20-21 lies beyond line 2 of the live `sdk:c.rs`"),
            "range: {error}"
        );
        assert_eq!(before, workspace.snapshot(), "range: nothing written");

        workspace.write_live("c.rs", b"");
        let error = format!(
            "{:#}",
            repair(&workspace.deck(), false, &exam()).unwrap_err()
        );
        assert!(
            error.contains("`sdk:c.rs`: the file is empty"),
            "empty: {error}"
        );
        assert_eq!(before, workspace.snapshot(), "empty: nothing written");

        workspace.write_live("c.rs", C);
        fs::remove_file(workspace.live().join("a.rs")).unwrap();
        let report = repair(&workspace.deck(), false, &exam()).unwrap();
        assert_eq!(
            (vec!["sdk:a.rs".to_string()], Vec::new()),
            (report.missing, report.refrozen),
            "missing: reported, never removed"
        );
        assert_eq!(
            before,
            workspace.snapshot(),
            "missing: the row and copy stay"
        );

        workspace.write_live("c.rs", b"edited\n");
        let mut roots = Roots::load(&workspace.dir()).unwrap().unwrap();
        roots.roots.insert("sdk".to_string(), PathBuf::from("src"));
        roots.save(&workspace.dir()).unwrap();
        let before = workspace.snapshot();
        let report = repair(&workspace.deck(), false, &exam()).unwrap();
        assert_eq!(
            ["sdk:a.rs", "sdk:c.rs"]
                .map(|path| {
                    (
                        path.to_string(),
                        "`sdk`: root `src` is not an absolute path (alix deck source root set \
                         deck-fixture sdk <dir>)"
                            .to_string(),
                    )
                })
                .to_vec(),
            report.skipped,
            "a root that is not an absolute folder is never used, and the reason names the fix"
        );
        assert_eq!(before, workspace.snapshot(), "unchecked: nothing written");
        let listed: Vec<(Drift, Option<String>)> = list(&workspace.deck(), &exam())
            .unwrap()
            .sources
            .into_iter()
            .map(|line| (line.drift, line.root_issue))
            .collect();
        assert_eq!(
            vec![
                (
                    Drift::Unchecked,
                    Some("root `src` is not an absolute path".to_string())
                );
                2
            ],
            listed,
            "list: a relative root never resolves against the cwd"
        );
    }

    /// A deck whose cards carry ids, with the named live files frozen as one
    /// source each and bound to `topics`; every sourced topic passed in a full
    /// sitting, the standing stamped, and the progress document saved.
    fn examined(files: &[&str], topics: &[&str]) -> Workspace {
        let workspace = Workspace::new();
        fs::write(workspace.deck(), EXAMINED_DECK).unwrap();
        workspace.write_live("a.rs", A);
        workspace.write_live("c.rs", C);
        workspace.add(files, topics).unwrap();
        let deck = Deck::load(workspace.deck()).unwrap();
        let mut store = state::open_store(&workspace.deck(), workspace.root()).unwrap();
        store.record_full_marks(&deck, NOW);
        assert!(
            store.deck_passed(DECK),
            "setup: the sitting passed the deck"
        );
        store.save().unwrap();
        workspace
    }

    fn progress_path(workspace: &Workspace) -> PathBuf {
        state::UserFiles::new(workspace.root()).progress_for(DECK)
    }

    fn reopen(workspace: &Workspace) -> (Deck, Store) {
        let deck = Deck::load(workspace.deck()).unwrap();
        let store = state::open_store(&workspace.deck(), workspace.root()).unwrap();
        (deck, store)
    }

    fn statuses(deck: &Deck, store: &Store) -> Vec<TopicStatus> {
        exam_status::exam_status(deck, store).topics
    }

    fn exam_changed(deck: &Deck, store: &Store) -> bool {
        exam_status::exam_row(deck, store, None).exam_changed
    }

    fn current_fingerprint(deck: &Deck, topic: &str) -> String {
        exam_status::examined_topics(deck)
            .into_iter()
            .find(|examined| examined.topic_id == topic)
            .map(|examined| examined.fingerprint)
            .unwrap_or_else(|| panic!("{topic} is sourced"))
    }

    fn results_of(workspace: &Workspace) -> BTreeMap<String, String> {
        reopen(workspace)
            .1
            .topic_results(DECK)
            .map(|results| {
                results
                    .iter()
                    .map(|(topic, result)| (topic.clone(), result.fingerprint.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn manifest_of(workspace: &Workspace) -> Manifest {
        load_verified(workspace.root(), DECK)
            .unwrap()
            .expect("a manifest survives")
            .manifest
    }

    fn rewrite_deck(workspace: &Workspace, from: &str, to: &str) {
        let text = fs::read_to_string(workspace.deck()).unwrap();
        assert!(text.contains(from), "deck holds {from:?}: {text}");
        fs::write(workspace.deck(), text.replacen(from, to, 1)).unwrap();
    }

    fn update_a(workspace: &Workspace, keep_results: bool) -> Result<UpdateReport> {
        update(
            &workspace.deck(),
            &source_id(A),
            &workspace.live().join("a.rs"),
            None,
            keep_results,
            &exam(),
        )
    }

    #[test]
    fn keep_results_rekeys_and_keeps_the_standing_solid() {
        let workspace = examined(&["a.rs"], &[TOPIC_A]);
        let before = results_of(&workspace)[TOPIC_A].clone();
        workspace.write_live("a.rs", EDITED);

        update_a(&workspace, true).unwrap_or_else(|error| panic!("update: {error:#}"));

        let (deck, store) = reopen(&workspace);
        let current = current_fingerprint(&deck, TOPIC_A);
        assert_ne!(
            before, current,
            "control: the refreeze changed the fingerprint"
        );
        assert_eq!(
            Some(&current),
            results_of(&workspace).get(TOPIC_A),
            "the result is re-keyed to the new fingerprint"
        );
        assert_eq!(
            vec![TopicStatus::Passed],
            statuses(&deck, &store),
            "the topic stays passed"
        );
        assert!(
            !exam_changed(&deck, &store),
            "the stamp is rewritten, so the standing is solid"
        );
    }

    #[test]
    fn keep_results_never_revives_a_result_already_stale_before_the_command() {
        let workspace = examined(&["a.rs"], &[TOPIC_A]);
        let f0 = results_of(&workspace)[TOPIC_A].clone();
        bind(
            &workspace.deck(),
            &source_id(A),
            &strings(&["2-4|topic-alpha"]),
            &exam(),
        )
        .unwrap();
        let f1 = current_fingerprint(&Deck::load(workspace.deck()).unwrap(), TOPIC_A);
        assert_ne!(f0, f1, "setup: the ranged binding changed the fingerprint");
        workspace.write_live("a.rs", EDITED);

        update_a(&workspace, true).unwrap_or_else(|error| panic!("update: {error:#}"));

        let (deck, store) = reopen(&workspace);
        let f2 = current_fingerprint(&deck, TOPIC_A);
        assert_ne!(
            f1, f2,
            "control: the refreeze changed the fingerprint again"
        );
        assert_eq!(
            Some(&f0),
            results_of(&workspace).get(TOPIC_A),
            "the result stale at F0 is left at F0"
        );
        assert_eq!(
            vec![TopicStatus::Unexamined],
            statuses(&deck, &store),
            "a stale result is never revived"
        );
        assert!(
            exam_changed(&deck, &store),
            "the standing was not solid before the command, so it stays dotted"
        );
    }

    #[test]
    fn keep_results_with_another_pending_change_rekeys_but_keeps_exam_changed() {
        let workspace = examined(&["a.rs"], &[TOPIC_A]);
        rewrite_deck(
            &workspace,
            "## q4\na4\n<!-- id: card-q4 -->\n",
            "## q4\na4\n<!-- id: card-q4 -->\n\n## q5\na5\n<!-- id: card-q5 -->\n",
        );
        workspace.write_live("a.rs", EDITED);

        update_a(&workspace, true).unwrap_or_else(|error| panic!("update: {error:#}"));

        let (deck, store) = reopen(&workspace);
        assert_eq!(
            vec![TopicStatus::Passed],
            statuses(&deck, &store),
            "the result is re-keyed"
        );
        assert!(
            exam_changed(&deck, &store),
            "a card added to an unsourced topic after the sitting keeps the standing dotted"
        );
    }

    #[test]
    fn keep_results_keeps_a_standing_solid_after_an_unsourced_card_was_removed() {
        let workspace = examined(&["a.rs"], &[TOPIC_A]);
        rewrite_deck(&workspace, "## q4\na4\n<!-- id: card-q4 -->\n", "");
        workspace.write_live("a.rs", EDITED);

        update_a(&workspace, true).unwrap_or_else(|error| panic!("update: {error:#}"));

        let (deck, store) = reopen(&workspace);
        assert_eq!(
            vec![TopicStatus::Passed],
            statuses(&deck, &store),
            "the result is re-keyed"
        );
        assert!(
            !exam_changed(&deck, &store),
            "the current unsourced cards are a subset of the recorded ones, so the stamp is \
             rewritten"
        );
    }

    #[test]
    fn update_without_keep_results_leaves_the_topic_unexamined() {
        let workspace = examined(&["a.rs"], &[TOPIC_A]);
        let f0 = results_of(&workspace)[TOPIC_A].clone();
        let store_bytes = fs::read(progress_path(&workspace)).unwrap();
        workspace.write_live("a.rs", EDITED);

        update_a(&workspace, false).unwrap_or_else(|error| panic!("update: {error:#}"));

        let (deck, store) = reopen(&workspace);
        assert_eq!(
            Some(&f0),
            results_of(&workspace).get(TOPIC_A),
            "the result keeps its old fingerprint"
        );
        assert_eq!(
            vec![TopicStatus::Unexamined],
            statuses(&deck, &store),
            "the topic reads unexamined"
        );
        assert!(exam_changed(&deck, &store), "the standing is dotted");
        assert_eq!(
            store_bytes,
            fs::read(progress_path(&workspace)).unwrap(),
            "no store write at all"
        );
    }

    #[test]
    fn repair_keep_results_rekeys_from_whole_command_snapshots() {
        let workspace = examined(&["a.rs", "c.rs"], &[TOPIC_A]);
        let before = results_of(&workspace)[TOPIC_A].clone();
        workspace.write_live("a.rs", EDITED);
        workspace.write_live("c.rs", b"1\n2\n3, edited\n");

        let report = repair(&workspace.deck(), true, &exam())
            .unwrap_or_else(|error| panic!("repair: {error:#}"));

        assert_eq!(
            2,
            report.refrozen.len(),
            "both drifted rows refrozen in one repair"
        );
        let (deck, store) = reopen(&workspace);
        let current = current_fingerprint(&deck, TOPIC_A);
        assert_ne!(
            before, current,
            "control: the repair changed the fingerprint"
        );
        assert_eq!(
            Some(&current),
            results_of(&workspace).get(TOPIC_A),
            "the result is re-keyed from the pre-command fingerprint to the post-command one"
        );
        assert_eq!(
            vec![TopicStatus::Passed],
            statuses(&deck, &store),
            "the topic stays passed"
        );
        assert!(!exam_changed(&deck, &store), "the standing is solid");
    }

    #[test]
    fn keep_results_refuses_a_progress_document_the_loader_refuses_before_writing_anything() {
        let workspace = examined(&["a.rs"], &[TOPIC_A]);
        fs::write(progress_path(&workspace), b"{").unwrap();
        let sources_before = workspace.snapshot();
        let store_bytes = fs::read(progress_path(&workspace)).unwrap();
        workspace.write_live("a.rs", EDITED);

        let error = update_a(&workspace, true).expect_err("a refused document fails the command");

        assert!(
            error.downcast_ref::<StoreError>().is_some(),
            "the loader's refusal is the error: {error:#}"
        );
        assert_eq!(
            sources_before,
            workspace.snapshot(),
            "the manifest and the copies are untouched"
        );
        assert_eq!(
            store_bytes,
            fs::read(progress_path(&workspace)).unwrap(),
            "the store is untouched"
        );
    }

    #[test]
    fn a_same_bytes_update_with_keep_results_rekeys_nothing_and_writes_no_store() {
        let workspace = examined(&["a.rs"], &[TOPIC_A]);
        rewrite_deck(&workspace, "## q4\na4\n<!-- id: card-q4 -->\n", "");
        let store_bytes = fs::read(progress_path(&workspace)).unwrap();

        update_a(&workspace, true).unwrap_or_else(|error| panic!("update: {error:#}"));

        assert_eq!(
            store_bytes,
            fs::read(progress_path(&workspace)).unwrap(),
            "no fingerprint moved, so nothing is re-keyed and the store is not written"
        );
    }

    #[test]
    fn keep_results_names_the_topics_it_could_not_keep_when_the_store_write_fails() {
        let scenario = || {
            let workspace = examined(&["a.rs"], &[TOPIC_A, TOPIC_B]);
            workspace.write_live("a.rs", EDITED);
            workspace
        };
        let reference = scenario();
        let manifest_before = manifest_of(&reference);
        let results_before = results_of(&reference);
        update_a(&reference, true).unwrap_or_else(|error| panic!("reference: {error:#}"));
        let manifest_after = manifest_of(&reference);
        let results_after = results_of(&reference);
        assert_ne!(
            manifest_before, manifest_after,
            "setup: the update changes the manifest"
        );
        assert_ne!(
            results_before, results_after,
            "setup: the update re-keys both results"
        );

        let mut rows = [false; 4];
        for n in 1.. {
            let workspace = scenario();
            let store_bytes = fs::read(progress_path(&workspace)).unwrap();
            let fault = crate::fsio::fault::fail_on_nth_operation(n);
            let result = update_a(&workspace, true);
            let triggered = fault.triggered_operation();
            drop(fault);
            let Some(operation) = triggered else {
                result.unwrap_or_else(|error| panic!("uninjected run {n}: {error:#}"));
                assert_eq!(
                    (&manifest_after, &results_after),
                    (&manifest_of(&workspace), &results_of(&workspace)),
                    "uninjected run: manifest new, results re-keyed"
                );
                break;
            };
            let step = format!("fault {n} ({operation:?})");
            let error = match result {
                Ok(_) => panic!("{step}: the fault was swallowed"),
                Err(error) => error,
            };
            let manifest = manifest_of(&workspace);
            let manifest_new = if manifest == manifest_before {
                false
            } else if manifest == manifest_after {
                true
            } else {
                panic!("{step}: the manifest is neither old nor new");
            };
            let results = results_of(&workspace);
            let results_new = if results == results_before {
                false
            } else if results == results_after {
                true
            } else {
                panic!("{step}: the results are neither old nor new: {results:?}");
            };
            let row = match (
                manifest_new,
                results_new,
                error.downcast_ref::<CommitError>(),
                error.downcast_ref::<KeepResultsNotSaved>(),
            ) {
                (false, false, Some(commit), None) if !commit.manifest_replaced => 1,
                (true, false, Some(commit), None) if commit.manifest_replaced => {
                    assert_eq!(
                        store_bytes,
                        fs::read(progress_path(&workspace)).unwrap(),
                        "{step}: row 2 never touches the store file"
                    );
                    2
                }
                (true, false, None, Some(not_saved)) => {
                    assert_eq!(
                        strings(&["Alpha", "Beta"]),
                        not_saved.topics,
                        "{step}: row 3 names both topics"
                    );
                    assert_eq!(
                        "results not kept for Alpha, Beta",
                        error.to_string(),
                        "{step}: the message names the topics only"
                    );
                    assert!(
                        error
                            .chain()
                            .nth(1)
                            .is_some_and(|cause| cause.to_string().contains("injected fault")),
                        "{step}: the store error travels as the source: {error:#}"
                    );
                    3
                }
                (true, true, None, Some(_)) => 4,
                (manifest_new, results_new, commit, not_saved) => panic!(
                    "{step}: fits no row: manifest_new={manifest_new} results_new={results_new} \
                     commit={commit:?} not_saved={not_saved:?}: {error:#}"
                ),
            };
            rows[row - 1] = true;
        }
        assert!(
            rows[2],
            "some fault lands in row 3 (store write failed, results unchanged): {rows:?}"
        );
        if cfg!(unix) {
            assert!(
                rows[1],
                "some fault lands in row 2 (manifest durability after its rename): {rows:?}"
            );
        }
    }

    #[test]
    fn resolve_deck_takes_a_file_then_a_unique_id_prefix_and_refuses_ambiguity() {
        let workspace = Workspace::new();
        fs::write(
            workspace.root().join("decks").join("e.md"),
            "---\nid: \"deck-fixtureb\"\n---\n## q\na\n",
        )
        .unwrap();
        let decks = workspace.root().join("decks");
        assert_eq!(
            workspace.deck(),
            resolve_deck("decks/d.md", workspace.root(), None).unwrap(),
            "an existing file wins"
        );
        assert_eq!(
            workspace.deck(),
            resolve_deck(DECK, &decks, None).unwrap(),
            "an exact id wins over a longer one"
        );
        assert_eq!(
            decks.join("e.md"),
            resolve_deck("deck-fixtureb", workspace.root(), None).unwrap()
        );
        let error = resolve_deck("deck-fix", workspace.root(), None).unwrap_err();
        assert!(
            error.to_string().contains("`deck-fix` matches 2 decks"),
            "{error}"
        );
        let elsewhere = tempfile::tempdir().unwrap();
        assert_eq!(
            decks.join("e.md"),
            resolve_deck("deck-fixtureb", elsewhere.path(), Some(workspace.root())).unwrap(),
            "the configured decks folder is searched"
        );
        let error = resolve_deck("deck-zzz", workspace.root(), None).unwrap_err();
        assert_eq!("no deck matches `deck-zzz`", error.to_string());
    }

    #[test]
    fn resolve_deck_refuses_a_full_id_held_by_two_workspaces_and_reads_only_the_frontmatter() {
        let decks_dir = tempfile::tempdir().unwrap();
        let mut copies = Vec::new();
        for name in ["one", "two"] {
            let members = decks_dir.path().join(name).join("decks");
            fs::create_dir_all(&members).unwrap();
            fs::write(decks_dir.path().join(name).join("alix.toml"), "").unwrap();
            let mut text = b"---\nid: \"deck-copied\"\n---\n## q\n".to_vec();
            text.extend_from_slice(&[0xff, 0xfe, b'\n']);
            fs::write(members.join("d.md"), text).unwrap();
            copies.push(members.join("d.md"));
        }
        let elsewhere = tempfile::tempdir().unwrap();
        let error = resolve_deck("deck-copied", elsewhere.path(), Some(decks_dir.path()))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("`deck-copied` matches 2 decks"),
            "an exact id on two decks is refused: {error}"
        );
        for copy in &copies {
            assert!(
                error.contains(&copy.display().to_string()),
                "the refusal names {}: {error}",
                copy.display()
            );
        }

        fs::remove_file(&copies[1]).unwrap();
        assert_eq!(
            copies[0],
            resolve_deck("deck-copied", elsewhere.path(), Some(decks_dir.path())).unwrap(),
            "a body that is not UTF-8 is never read"
        );
    }
}
