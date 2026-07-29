//! The Catalog owner: root inputs, deck cache, launcher images, recent
//! history, complete `DeckListDto` builds, and the resolution snapshot that
//! maps accepted client names to validated targets. Resolution is a discovery
//! product rebuilt only when the cheap root metadata drifts or an Alix write
//! invalidates it; it never computes row status. Per-file content staleness
//! is `DeckCache`'s own (mtime, size) validation.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
    time::SystemTime,
};

use super::{catalog::*, dto::DeckListDto};
use crate::{
    cache::DeckCache, config::ReviewConfig, recent::RecentDecks, session::now_ms, store::Store,
    workspace,
};

pub(super) struct CatalogConfig {
    pub(super) scoped: bool,
    pub(super) config_path: Option<PathBuf>,
    pub(super) review_cfg: ReviewConfig,
}

pub(super) struct CatalogState {
    pub(super) config: CatalogConfig,
    pub(super) decks_dir: PathBuf,
    pub(super) cache: DeckCache,
    pub(super) launcher_icons: HashMap<String, PathBuf>,
    pub(super) recent: RecentDecks,
    resolution: Option<CachedResolution>,
    rebuilds: u64,
    // One in-flight complete build at a time: the first requester leads and
    // builds on its own worker thread from cloned inputs, later requesters
    // wait on its result, and the owner stays free to resolve names.
    build: Option<InFlightBuild>,
    build_generation: u64,
    builds_merged: u64,
}

struct InFlightBuild {
    generation: u64,
    waiters: Vec<Reply<BuildWait>>,
}

/// Cloned inputs a leader builds from. The memo caches merge back on finish
/// (last-writer-wins is correct for (mtime, size)-validated derivations);
/// recent history and the root never write back from a build.
pub(super) struct BuildInputs {
    pub(super) generation: u64,
    pub(super) decks_dir: PathBuf,
    pub(super) recent: RecentDecks,
    pub(super) cache: DeckCache,
    pub(super) launcher_icons: HashMap<String, PathBuf>,
    pub(super) review_cfg: ReviewConfig,
}

pub(super) enum BuildStart {
    Lead(Box<BuildInputs>),
    Join(mpsc::Receiver<BuildWait>),
}

pub(super) enum BuildWait {
    Ready(Result<DeckListDto, String>),
    // The inputs were invalidated mid-build; the completion is discarded
    // and the requester begins again.
    Retry,
}

impl CatalogState {
    pub(super) fn new(config: CatalogConfig, decks_dir: PathBuf, recent: RecentDecks) -> Self {
        CatalogState {
            config,
            decks_dir,
            cache: DeckCache::default(),
            launcher_icons: HashMap::new(),
            recent,
            resolution: None,
            rebuilds: 0,
            build: None,
            build_generation: 0,
            builds_merged: 0,
        }
    }
}

struct CachedResolution {
    map: HashMap<String, Resolved>,
    // Directory rows by bare name, for destination resolution (a dest must
    // be a unique directory row).
    dirs: HashMap<String, Vec<PathBuf>>,
    meta: RootMeta,
}

/// The cheap discovery inputs whose drift invalidates the name map: the root
/// and its first-level entries, each container's member dir, and every recent
/// entry (recent candidates can live outside the root).
type RootMeta = Vec<(PathBuf, Option<(SystemTime, u64)>)>;

fn stat(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn root_meta(decks_dir: &Path, recent: &RecentDecks) -> RootMeta {
    let mut out: RootMeta = vec![(decks_dir.to_path_buf(), stat(decks_dir))];
    if let Ok(entries) = std::fs::read_dir(decks_dir) {
        let mut children: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        children.sort();
        for child in children {
            out.push((child.clone(), stat(&child)));
            if child.is_dir() {
                let members = workspace::member_dir(&child);
                if members != child {
                    out.push((members.clone(), stat(&members)));
                }
            }
        }
    }
    for entry in recent.entries() {
        out.push((entry.path.clone(), stat(&entry.path)));
    }
    out
}

pub(super) enum SetDeadlineError {
    BadTarget,
    WriteFailed,
}

type Reply<T> = mpsc::Sender<T>;

pub(super) enum CatalogCommand {
    Resolve {
        name: String,
        reply: Reply<Resolved>,
    },
    ResolveDest {
        name: Option<String>,
        reply: Reply<Option<PathBuf>>,
    },
    BeginList(Reply<BuildStart>),
    FinishList {
        generation: u64,
        result: Result<DeckListDto, String>,
        cache: DeckCache,
        launcher_icons: HashMap<String, PathBuf>,
        reply: Reply<BuildWait>,
    },
    SetDeadline {
        name: String,
        date: Option<chrono::NaiveDate>,
        reply: Reply<Result<(), SetDeadlineError>>,
    },
    RecordRecent {
        paths: Vec<PathBuf>,
    },
    InvalidateContent,
    LauncherIcon {
        key: String,
        reply: Reply<Option<PathBuf>>,
    },
    DecksRoot(Reply<PathBuf>),
}

#[derive(Clone)]
pub(super) struct CatalogHandle {
    tx: mpsc::Sender<CatalogCommand>,
}

impl CatalogHandle {
    fn call<R>(&self, build: impl FnOnce(Reply<R>) -> CatalogCommand) -> Option<R> {
        let (tx, rx) = mpsc::channel();
        self.tx.send(build(tx)).ok()?;
        rx.recv().ok()
    }

    pub(super) fn resolve(&self, name: String) -> Option<Resolved> {
        self.call(|reply| CatalogCommand::Resolve { name, reply })
    }
    pub(super) fn resolve_path(&self, name: String) -> Option<Option<PathBuf>> {
        self.resolve(name).map(resolved_path)
    }
    pub(super) fn resolve_dest(&self, name: Option<String>) -> Option<Option<PathBuf>> {
        self.call(|reply| CatalogCommand::ResolveDest { name, reply })
    }
    /// A complete listing: leads the build on this thread or waits on the
    /// in-flight leader's result; an invalidated completion retries.
    pub(super) fn list(&self, projection: Arc<Store>) -> Option<Result<DeckListDto, String>> {
        loop {
            match self.call(CatalogCommand::BeginList)? {
                BuildStart::Lead(inputs) => {
                    let mut inputs = *inputs;
                    let result = deck_catalog(
                        &inputs.decks_dir,
                        &inputs.recent,
                        &projection,
                        true,
                        &mut inputs.launcher_icons,
                        inputs.review_cfg,
                        &mut inputs.cache,
                    )
                    .map_err(|e| format!("{}: {e}", inputs.decks_dir.display()));
                    match self.call(|reply| CatalogCommand::FinishList {
                        generation: inputs.generation,
                        result,
                        cache: inputs.cache,
                        launcher_icons: inputs.launcher_icons,
                        reply,
                    })? {
                        BuildWait::Ready(result) => return Some(result),
                        BuildWait::Retry => continue,
                    }
                }
                BuildStart::Join(rx) => match rx.recv().ok()? {
                    BuildWait::Ready(result) => return Some(result),
                    BuildWait::Retry => continue,
                },
            }
        }
    }
    pub(super) fn set_deadline(
        &self,
        name: String,
        date: Option<chrono::NaiveDate>,
    ) -> Option<Result<(), SetDeadlineError>> {
        self.call(|reply| CatalogCommand::SetDeadline { name, date, reply })
    }
    pub(super) fn record_recent(&self, paths: Vec<PathBuf>) {
        let _ = self.tx.send(CatalogCommand::RecordRecent { paths });
    }
    pub(super) fn invalidate_content(&self) {
        let _ = self.tx.send(CatalogCommand::InvalidateContent);
    }
    pub(super) fn launcher_icon(&self, key: String) -> Option<Option<PathBuf>> {
        self.call(|reply| CatalogCommand::LauncherIcon { key, reply })
    }
    pub(super) fn decks_root(&self) -> Option<PathBuf> {
        self.call(CatalogCommand::DecksRoot)
    }
}

pub(super) fn spawn(
    failure: super::OwnerFailure,
    state: CatalogState,
) -> (CatalogHandle, thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel();
    let handle = super::supervised(failure, move || {
        let mut state = state;
        for cmd in rx {
            state.handle(cmd);
        }
    });
    (CatalogHandle { tx }, handle)
}

impl CatalogState {
    fn handle(&mut self, cmd: CatalogCommand) {
        match cmd {
            CatalogCommand::Resolve { name, reply } => {
                let _ = reply.send(self.resolve(&name));
            }
            CatalogCommand::ResolveDest { name, reply } => {
                let _ = reply.send(self.resolve_dest(name.as_deref()));
            }
            CatalogCommand::BeginList(reply) => {
                let _ = reply.send(self.begin_list());
            }
            CatalogCommand::FinishList {
                generation,
                result,
                cache,
                launcher_icons,
                reply,
            } => {
                let out = self.finish_list(generation, result, cache, launcher_icons);
                let _ = reply.send(out);
            }
            CatalogCommand::SetDeadline { name, date, reply } => {
                let _ = reply.send(self.set_deadline(&name, date));
            }
            CatalogCommand::RecordRecent { paths } => {
                self.recent.record(&paths, now_ms());
                let _ = self.recent.save();
                // Recent candidates feed the name map (they can live outside
                // the root); the mapping for unrelated names is unaffected
                // because the rebuild re-derives every target from disk.
                self.invalidate();
            }
            CatalogCommand::InvalidateContent => {
                self.invalidate();
            }
            CatalogCommand::LauncherIcon { key, reply } => {
                let _ = reply.send(self.launcher_icons.get(&key).cloned());
            }
            CatalogCommand::DecksRoot(reply) => {
                let _ = reply.send(self.decks_dir.clone());
            }
        }
    }

    fn refresh_root(&mut self) {
        let dir = effective_decks_dir(
            self.config.scoped,
            self.config.config_path.as_deref(),
            &self.decks_dir,
        );
        if dir != self.decks_dir {
            self.decks_dir = dir;
            self.resolution = None;
        }
    }

    fn ensure_resolution(&mut self) {
        self.refresh_root();
        let meta = root_meta(&self.decks_dir, &self.recent);
        if self
            .resolution
            .as_ref()
            .is_some_and(|cached| cached.meta == meta)
        {
            return;
        }
        // A root error resolves nothing and is never cached, so the next
        // request retries the filesystem instead of trusting a bad snapshot.
        match resolution_maps(&self.decks_dir, &self.recent, &mut self.cache) {
            Ok(maps) => {
                self.resolution = Some(CachedResolution {
                    map: maps.map,
                    dirs: maps.dirs,
                    meta,
                });
            }
            Err(_) => self.resolution = None,
        }
        self.rebuilds += 1;
    }

    fn resolve(&mut self, name: &str) -> Resolved {
        self.ensure_resolution();
        self.resolution
            .as_ref()
            .and_then(|cached| cached.map.get(name).cloned())
            .unwrap_or(Resolved::Unknown)
    }

    fn resolve_dest(&mut self, name: Option<&str>) -> Option<PathBuf> {
        let Some(name) = name.filter(|d| !d.is_empty()) else {
            self.refresh_root();
            return Some(workspace::member_dir(&self.decks_dir));
        };
        self.ensure_resolution();
        let dirs = self.resolution.as_ref()?.dirs.get(name)?;
        match dirs.as_slice() {
            [only] => Some(workspace::member_dir(only)),
            _ => None, // ambiguous: more than one dir row shares this name
        }
    }

    fn invalidate(&mut self) {
        self.resolution = None;
        self.build_generation = self.build_generation.wrapping_add(1);
    }

    fn begin_list(&mut self) -> BuildStart {
        if let Some(build) = self.build.as_mut() {
            let (tx, rx) = mpsc::channel();
            build.waiters.push(tx);
            return BuildStart::Join(rx);
        }
        self.refresh_root();
        self.build = Some(InFlightBuild {
            generation: self.build_generation,
            waiters: Vec::new(),
        });
        BuildStart::Lead(Box::new(BuildInputs {
            generation: self.build_generation,
            decks_dir: self.decks_dir.clone(),
            recent: self.recent.clone(),
            cache: self.cache.clone(),
            launcher_icons: self.launcher_icons.clone(),
            review_cfg: self.config.review_cfg,
        }))
    }

    fn finish_list(
        &mut self,
        generation: u64,
        result: Result<DeckListDto, String>,
        cache: DeckCache,
        launcher_icons: HashMap<String, PathBuf>,
    ) -> BuildWait {
        let Some(build) = self.build.take() else {
            return BuildWait::Retry;
        };
        if build.generation != generation || generation != self.build_generation {
            // Invalidated mid-build: discard the completion (plan phase 5)
            // and send everyone back to begin again.
            for waiter in build.waiters {
                let _ = waiter.send(BuildWait::Retry);
            }
            return BuildWait::Retry;
        }
        self.cache = cache;
        self.launcher_icons = launcher_icons;
        self.builds_merged += 1;
        for waiter in build.waiters {
            let _ = waiter.send(BuildWait::Ready(result.clone()));
        }
        BuildWait::Ready(result)
    }

    fn set_deadline(
        &mut self,
        name: &str,
        date: Option<chrono::NaiveDate>,
    ) -> Result<(), SetDeadlineError> {
        let dir = match self.resolve(name) {
            Resolved::Many { dir, .. } if crate::workspace::is_workspace(&dir) => dir,
            _ => return Err(SetDeadlineError::BadTarget),
        };
        if let Err(e) = crate::workspace::set_deadline(&dir, date) {
            eprintln!("workspace deadline write failed: {e:#}");
            return Err(SetDeadlineError::WriteFailed);
        }
        self.invalidate();
        Ok(())
    }

    #[cfg(test)]
    fn rebuild_count(&self) -> u64 {
        self.rebuilds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_deck(path: &Path, stem: &str) {
        std::fs::write(path, format!("---\nid: \"deck-{stem}\"\n---\n## f\nb\n")).unwrap();
    }

    fn state_over(dir: &Path) -> CatalogState {
        CatalogState::new(
            CatalogConfig {
                scoped: true,
                config_path: None,
                review_cfg: ReviewConfig::default(),
            },
            dir.to_path_buf(),
            RecentDecks::load(dir.join("recent.json")),
        )
    }

    #[test]
    fn a_second_list_joins_the_in_flight_build_and_gets_its_result() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(&dir.path().join("alpha.md"), "alpha");
        let mut s = state_over(dir.path());

        let BuildStart::Lead(inputs) = s.begin_list() else {
            panic!("the first requester leads");
        };
        let BuildStart::Join(rx) = s.begin_list() else {
            panic!("the second requester joins the in-flight build");
        };
        let done = s.finish_list(
            inputs.generation,
            Ok(DeckListDto {
                workspaces: Vec::new(),
                recent: Vec::new(),
                folders: Vec::new(),
            }),
            inputs.cache,
            inputs.launcher_icons,
        );
        assert!(matches!(done, BuildWait::Ready(Ok(_))));
        assert!(matches!(rx.try_recv(), Ok(BuildWait::Ready(Ok(_)))));
        assert_eq!(1, s.builds_merged);
    }

    #[test]
    fn an_invalidation_mid_build_discards_the_completion_and_retries() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(&dir.path().join("alpha.md"), "alpha");
        let mut s = state_over(dir.path());

        let BuildStart::Lead(inputs) = s.begin_list() else {
            panic!("the first requester leads");
        };
        let BuildStart::Join(rx) = s.begin_list() else {
            panic!("the second requester joins");
        };
        s.invalidate();
        let done = s.finish_list(
            inputs.generation,
            Ok(DeckListDto {
                workspaces: Vec::new(),
                recent: Vec::new(),
                folders: Vec::new(),
            }),
            inputs.cache,
            inputs.launcher_icons,
        );
        assert!(matches!(done, BuildWait::Retry));
        assert!(matches!(rx.try_recv(), Ok(BuildWait::Retry)));
        assert_eq!(0, s.builds_merged, "an invalidated completion never merges");
    }

    #[test]
    fn resolution_answers_while_a_complete_build_is_in_flight() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(&dir.path().join("alpha.md"), "alpha");
        let mut s = state_over(dir.path());

        let BuildStart::Lead(_inputs) = s.begin_list() else {
            panic!("the first requester leads");
        };
        // The owner is not building anything itself: a resolve between
        // begin and finish answers from the snapshot immediately.
        assert!(matches!(s.resolve("alpha.md"), Resolved::One(_)));
    }

    #[test]
    fn warm_resolution_reuses_the_map_without_a_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(&dir.path().join("alpha.md"), "alpha");
        let mut s = state_over(dir.path());

        assert!(matches!(s.resolve("alpha.md"), Resolved::One(_)));
        assert!(matches!(s.resolve("alpha.md"), Resolved::One(_)));
        assert_eq!(1, s.rebuild_count());
    }

    #[test]
    fn an_external_file_addition_rebuilds_the_map_on_the_next_request() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(&dir.path().join("alpha.md"), "alpha");
        let mut s = state_over(dir.path());
        assert!(matches!(s.resolve("beta.md"), Resolved::Unknown));

        write_deck(&dir.path().join("beta.md"), "beta");

        assert!(matches!(s.resolve("beta.md"), Resolved::One(_)));
        assert_eq!(2, s.rebuild_count());
    }

    #[test]
    fn external_rename_refreshes_resolution_on_the_next_request() {
        let dir = tempfile::tempdir().unwrap();
        write_deck(&dir.path().join("alpha.md"), "alpha");
        let mut s = state_over(dir.path());
        assert!(matches!(s.resolve("alpha.md"), Resolved::One(_)));

        std::fs::rename(dir.path().join("alpha.md"), dir.path().join("omega.md")).unwrap();

        assert!(matches!(s.resolve("alpha.md"), Resolved::Unknown));
        assert!(matches!(s.resolve("omega.md"), Resolved::One(_)));
    }

    #[test]
    fn recent_order_invalidation_preserves_target_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let alpha = dir.path().join("alpha.md");
        let beta = dir.path().join("beta.md");
        write_deck(&alpha, "alpha");
        write_deck(&beta, "beta");
        let mut s = state_over(dir.path());
        let before = s.resolve("alpha.md");

        s.recent.record(std::slice::from_ref(&beta), 1000);
        s.resolution = None;

        assert_eq!(before, s.resolve("alpha.md"));
        assert_eq!(2, s.rebuild_count());
    }

    #[test]
    fn two_rows_sharing_one_name_resolve_ambiguous_never_one_of_them() {
        // Container members carry qualified names, so the colliding pair is
        // a root deck plus a recent entry outside the root with the same
        // file name.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("decks");
        std::fs::create_dir(&root).unwrap();
        write_deck(&root.join("same.md"), "aa");
        let outside = dir.path().join("same.md");
        write_deck(&outside, "bb");
        let mut s = state_over(&root);
        s.recent.record(std::slice::from_ref(&outside), 1000);

        assert!(matches!(s.resolve("same.md"), Resolved::Ambiguous));
    }

    #[test]
    fn a_failed_root_is_not_cached_and_recovers_after_restore() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("decks");
        std::fs::create_dir(&root).unwrap();
        write_deck(&root.join("alpha.md"), "alpha");
        let mut s = state_over(&root);
        assert!(matches!(s.resolve("alpha.md"), Resolved::One(_)));

        let away = dir.path().join("away");
        std::fs::rename(&root, &away).unwrap();
        assert!(matches!(s.resolve("alpha.md"), Resolved::Unknown));

        std::fs::rename(&away, &root).unwrap();
        assert!(matches!(s.resolve("alpha.md"), Resolved::One(_)));
    }

    #[test]
    fn a_member_added_inside_a_workspace_member_dir_is_discovered() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("box");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.join(crate::workspace::MANIFEST), "title = \"B\"\n").unwrap();
        let members = crate::workspace::member_dir(&ws);
        std::fs::create_dir_all(&members).unwrap();
        write_deck(&members.join("one.md"), "one");
        let mut s = state_over(dir.path());
        assert!(matches!(s.resolve("box/one.md"), Resolved::One(_)));

        write_deck(&members.join("two.md"), "two");

        assert!(matches!(s.resolve("box/two.md"), Resolved::One(_)));
    }
}
