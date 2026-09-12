use std::path::{Path, PathBuf};

use alix::paired::{self, Choice, PairedRoot, PushOutcome};
use anyhow::Result;

pub struct PairedDeckState {
    pub deck_id: String,
    pub path: String,
    pub unpushed: bool,
    pub phone_saves: u64,
    pub phone_at_ms: Option<u64>,
    pub conflict: Option<PairedConflict>,
}

pub struct PairedEntryState {
    pub entry: String,
    pub kind: String,
    pub digest: String,
    pub decks: Vec<PairedDeckState>,
}

pub struct PairedWriter {
    pub device: String,
    pub at_ms: u64,
}

pub enum PairedConflict {
    Push {
        desktop_revision: Option<u64>,
        pulled_revision: Option<u64>,
        desktop_writer: Option<PairedWriter>,
    },
    Pull {
        pulled_revision: Option<u64>,
        pulled_writer: Option<PairedWriter>,
    },
}

pub struct PushPlanItem {
    pub deck_id: String,
    pub entry: String,
    pub document: String,
    pub base: Option<u64>,
    pub phone_revision: u64,
}

pub enum PushOutcomeDto {
    Accepted {
        revision: u64,
    },
    Conflict {
        desktop_revision: Option<u64>,
        desktop_writer: Option<PairedWriter>,
    },
    NotServed,
}

pub enum ResolutionDto {
    Done,
    Push { item: PushPlanItem },
    Pull { entry: String },
}

pub struct PullReportDto {
    pub entry: String,
    pub kind: String,
    pub landed: Vec<String>,
    pub kept: Vec<String>,
    pub conflicts: Vec<String>,
    pub phone_only: Vec<String>,
    pub removed: Vec<String>,
}

pub struct RenamedEntry {
    pub old: String,
    pub new: String,
}

impl From<alix::store::Writer> for PairedWriter {
    fn from(writer: alix::store::Writer) -> Self {
        Self {
            device: writer.device,
            at_ms: writer.at_ms,
        }
    }
}

impl From<PairedWriter> for alix::store::Writer {
    fn from(writer: PairedWriter) -> Self {
        Self {
            device: writer.device,
            at_ms: writer.at_ms,
        }
    }
}

impl From<paired::Conflict> for PairedConflict {
    fn from(conflict: paired::Conflict) -> Self {
        match conflict {
            paired::Conflict::Push {
                desktop_revision,
                pulled_revision,
                desktop_writer,
            } => Self::Push {
                desktop_revision,
                pulled_revision,
                desktop_writer: desktop_writer.map(Into::into),
            },
            paired::Conflict::Pull {
                pulled_revision,
                pulled_writer,
            } => Self::Pull {
                pulled_revision,
                pulled_writer: pulled_writer.map(Into::into),
            },
        }
    }
}

impl From<paired::PushItem> for PushPlanItem {
    fn from(item: paired::PushItem) -> Self {
        Self {
            deck_id: item.deck_id,
            entry: item.entry,
            document: item.document.to_string_lossy().into_owned(),
            base: item.base,
            phone_revision: item.phone_revision,
        }
    }
}

impl From<PushPlanItem> for paired::PushItem {
    fn from(item: PushPlanItem) -> Self {
        Self {
            deck_id: item.deck_id,
            entry: item.entry,
            document: PathBuf::from(item.document),
            base: item.base,
            phone_revision: item.phone_revision,
        }
    }
}

impl From<paired::PullReport> for PullReportDto {
    fn from(report: paired::PullReport) -> Self {
        Self {
            entry: report.entry,
            kind: report.kind,
            landed: report.landed,
            kept: report.kept,
            conflicts: report.conflicts,
            phone_only: report.phone_only,
            removed: report.removed,
        }
    }
}

fn root(dir: &str) -> PairedRoot {
    PairedRoot::new(Path::new(dir))
}

#[flutter_rust_bridge::frb(sync)]
pub fn paired_root_dir(support: String, root_id: String) -> String {
    Path::new(&support)
        .join(paired::PAIRED_DIR)
        .join(root_id)
        .to_string_lossy()
        .into_owned()
}

#[flutter_rust_bridge::frb(sync)]
pub fn paired_recover(root_dir: String) -> Result<Vec<String>> {
    std::fs::create_dir_all(&root_dir)?;
    paired::recover(&root(&root_dir))
}

#[flutter_rust_bridge::frb(sync)]
pub fn paired_entries(root_dir: String) -> Result<Vec<PairedEntryState>> {
    Ok(paired::pulled_entries(&root(&root_dir))?
        .into_iter()
        .map(|entry| PairedEntryState {
            entry: entry.entry,
            kind: entry.kind,
            digest: entry.digest,
            decks: entry
                .decks
                .into_iter()
                .map(|deck| PairedDeckState {
                    deck_id: deck.deck_id,
                    path: deck.path,
                    unpushed: deck.unpushed,
                    phone_saves: deck.phone_saves,
                    phone_at_ms: deck.phone_at_ms,
                    conflict: deck.conflict.map(Into::into),
                })
                .collect(),
        })
        .collect())
}

#[flutter_rust_bridge::frb(sync)]
pub fn paired_plan_pushes(root_dir: String) -> Result<Vec<PushPlanItem>> {
    Ok(paired::plan_pushes(&root(&root_dir))?
        .into_iter()
        .map(Into::into)
        .collect())
}

#[flutter_rust_bridge::frb(sync)]
pub fn paired_record_push(
    root_dir: String,
    item: PushPlanItem,
    outcome: PushOutcomeDto,
) -> Result<()> {
    let outcome = match outcome {
        PushOutcomeDto::Accepted { revision } => PushOutcome::Accepted { revision },
        PushOutcomeDto::Conflict {
            desktop_revision,
            desktop_writer,
        } => PushOutcome::Conflict {
            desktop_revision,
            desktop_writer: desktop_writer.map(Into::into),
        },
        PushOutcomeDto::NotServed => PushOutcome::NotServed,
    };
    paired::record_push(&root(&root_dir), &item.into(), outcome)
}

/// Unpacks and lands one pulled entry; runs off the UI isolate (blocking
/// file work, so not `sync`).
pub fn paired_apply_pull(root_dir: String, entry: String, zip_path: String) -> Result<PullReportDto> {
    paired::apply_pull(&root(&root_dir), &entry, Path::new(&zip_path), &mut |_| Ok(()))
        .map(Into::into)
}

#[flutter_rust_bridge::frb(sync)]
pub fn paired_resolve_conflict(
    root_dir: String,
    deck_id: String,
    keep_phone: bool,
) -> Result<ResolutionDto> {
    let choice = if keep_phone {
        Choice::KeepPhone
    } else {
        Choice::TakeDesktop
    };
    Ok(match paired::resolve_conflict(&root(&root_dir), &deck_id, choice)? {
        paired::Resolution::Done => ResolutionDto::Done,
        paired::Resolution::Push(item) => ResolutionDto::Push { item: item.into() },
        paired::Resolution::Pull { entry } => ResolutionDto::Pull { entry },
    })
}

#[flutter_rust_bridge::frb(sync)]
pub fn paired_tidy_renamed(root_dir: String, listed: Vec<String>) -> Result<Vec<RenamedEntry>> {
    Ok(paired::tidy_renamed(&root(&root_dir), &listed)?
        .into_iter()
        .map(|(old, new)| RenamedEntry { old, new })
        .collect())
}

#[flutter_rust_bridge::frb(sync)]
pub fn paired_orphans(root_dir: String, listed: Vec<String>) -> Result<Vec<String>> {
    paired::orphans(&root(&root_dir), &listed)
}

#[flutter_rust_bridge::frb(sync)]
pub fn paired_staging_zip(root_dir: String, entry: String) -> String {
    root(&root_dir)
        .staging_zip(&entry)
        .to_string_lossy()
        .into_owned()
}

#[flutter_rust_bridge::frb(sync)]
pub fn paired_remove_entry(root_dir: String, entry: String) -> Result<()> {
    paired::remove_entry(&root(&root_dir), &entry)
}

#[flutter_rust_bridge::frb(sync)]
pub fn paired_free_space(path: String) -> Result<u64> {
    paired::free_space(Path::new(&path))
}

#[flutter_rust_bridge::frb(sync)]
pub fn paired_needs_space(compressed: u64, unpacked: u64) -> u64 {
    paired::needs_space(compressed, unpacked)
}
