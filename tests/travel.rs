//! One law over every lifecycle operation: each deck's per-deck folders
//! travel with the deck, and the machine-local roots file never does.
//! Rows are operations, columns are folder kinds; every assertion names its
//! cell.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use alix::{
    assets,
    augment::AugmentCache,
    config::ExamConfig,
    deck::Deck,
    deck_transfer::{self, TransferMode},
    explore, library,
    paired::{self, KIND_DECK, KIND_WORKSPACE, PairedRoot},
    share::{self, SourcesPolicy},
    sources::{
        self,
        commands::{self, AddRequest},
    },
    state,
    store::Store,
    sync::{SYNC_PULL_MANIFEST_VERSION, SyncDeckDto, SyncFileDto, SyncPullManifest, fingerprint},
};
use tempfile::TempDir;

const DECK_ID: &str = "deck-travel1";
const TOPIC_ID: &str = "topic-alpha1";
const CARD_ID: &str = "card-c1";
const DECK_FILE: &str = "facts.md";
const DECK_TEXT: &str = "---\nformat-version: 1\nid: \"deck-travel1\"\n---\n# Alpha\n<!-- id: topic-alpha1 \
                         -->\n\n## q\na\n<!-- id: card-c1 -->\n";
const SOURCE_BYTES: &[u8] = b"the source\nsecond line\n";
const ROOT_ID: &str = "root-9w2c7x4k1m8q3z5t0v6b2n4d8f";
const PHONE_ROOTS: &[u8] = b"version = 1\n\n[roots]\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Asset,
    Augment,
    Manifest,
    Copy,
    Roots,
    Progress,
}

const KINDS: [Kind; 6] = [
    Kind::Asset,
    Kind::Augment,
    Kind::Manifest,
    Kind::Copy,
    Kind::Roots,
    Kind::Progress,
];

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Asset => "assets/<id>/<object>",
            Kind::Augment => "augment/<id>.json",
            Kind::Manifest => "sources/<id>/manifest.toml",
            Kind::Copy => "sources/<id>/<copy>",
            Kind::Roots => "sources/<id>/manifest.local.toml",
            Kind::Progress => ".alix/progress/<id>.json",
        }
    }
}

/// The names the fixture minted for its content-addressed files.
struct Member {
    asset: String,
    copy: String,
}

impl Member {
    fn rel(&self, kind: Kind) -> String {
        match kind {
            Kind::Asset => format!("assets/{DECK_ID}/{}", self.asset),
            Kind::Augment => format!("augment/{DECK_ID}.json"),
            Kind::Manifest => format!("sources/{DECK_ID}/{}", sources::MANIFEST),
            Kind::Copy => format!("sources/{DECK_ID}/{}", self.copy),
            Kind::Roots => format!("sources/{DECK_ID}/{}", sources::ROOTS),
            Kind::Progress => format!(".alix/progress/{DECK_ID}.json"),
        }
    }
}

const ALL: [bool; 6] = [true; 6];
const NONE: [bool; 6] = [false; 6];
/// What a share with its sources carries: public material, never the roots
/// file, never progress.
const CARRIED: [bool; 6] = [true, true, true, true, false, false];
/// What a share without its sources carries.
const OMITTED: [bool; 6] = [true, true, false, false, false, false];

thread_local! {
    static FAILED_CELLS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Records every cell that disagrees, so one run shows the whole table;
/// [`assert_every_cell_held`] fails the test with the list.
fn check(op: &str, side: &str, root: &Path, member: &Member, expected: [bool; 6]) {
    for (kind, want) in KINDS.iter().zip(expected) {
        let rel = member.rel(*kind);
        let path = rel_path(root, &rel);
        let present = path.exists();
        if want != present {
            FAILED_CELLS.with(|cells| {
                cells.borrow_mut().push(format!(
                    "{op} / {} / {side}: {rel} present={present}, expected {want} (at {})",
                    kind.label(),
                    path.display()
                ));
            });
        }
    }
}

fn assert_every_cell_held() {
    let failed = FAILED_CELLS.with(|cells| std::mem::take(&mut *cells.borrow_mut()));
    assert!(
        failed.is_empty(),
        "{} cell(s) disagree:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

/// One extra assertion recorded as a cell, so the sweep keeps going.
fn cell(held: bool, message: String) {
    if !held {
        FAILED_CELLS.with(|cells| cells.borrow_mut().push(message));
    }
}

/// A fixture must be whole before an operation runs on it; earlier
/// operations' recorded cells stay recorded.
fn assert_fixture_whole(op: &str, root: &Path, member: &Member) {
    let before = FAILED_CELLS.with(|cells| cells.borrow().len());
    check(op, "workspace", root, member, ALL);
    let fresh: Vec<String> = FAILED_CELLS.with(|cells| cells.borrow()[before..].to_vec());
    assert!(fresh.is_empty(), "{op} is not whole:\n{}", fresh.join("\n"));
}

/// The receiver's own roots file, as `alix deck source root set` would leave it.
fn plant_roots(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, PHONE_ROOTS).unwrap();
}

fn rel_path(root: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

fn workspace_at(parent: &Path, name: &str) -> PathBuf {
    let root = parent.join(name);
    fs::create_dir_all(root.join("decks")).unwrap();
    fs::write(root.join("alix.toml"), format!("title = \"{name}\"\n")).unwrap();
    root
}

/// A workspace member with every per-deck folder kind populated: an owned
/// asset object, an augmentation document, a progress document, and one
/// frozen source bound to its topic with a saved root.
fn sourced_member(root: &Path) -> Member {
    let deck = root.join("decks").join(DECK_FILE);
    fs::write(&deck, DECK_TEXT).unwrap();
    let object = assets::write_object(root, DECK_ID, b"excerpt\n", "md").unwrap();
    let loaded = Deck::load(&deck).unwrap();
    let mut cache = AugmentCache::open_for_deck(&loaded).unwrap();
    cache.set_note(CARD_ID, "note".to_string(), 1);
    cache.save().unwrap();
    let mut store = state::open_store(&deck, root).unwrap();
    store.get_or_insert(CARD_ID);
    store.save().unwrap();
    let live = root.join("live");
    fs::create_dir_all(&live).unwrap();
    let file = live.join("notes.txt");
    fs::write(&file, SOURCE_BYTES).unwrap();
    let report = commands::add(
        AddRequest {
            deck: &deck,
            files: vec![file],
            root: Some(("sdk".to_string(), live)),
            key: None,
            topics: vec![TOPIC_ID.to_string()],
        },
        &ExamConfig::default(),
    )
    .unwrap();
    let member = Member {
        asset: object.file_name().unwrap().to_string_lossy().into_owned(),
        copy: format!("{}.txt", report.added[0].0),
    };
    assert_fixture_whole("fixture", root, &member);
    member
}

fn deck_path(root: &Path) -> PathBuf {
    root.join("decks").join(DECK_FILE)
}

/// A missing file is a recorded cell, not a stop, so the sweep keeps going.
fn read(root: &Path, rel: &str) -> Vec<u8> {
    let path = rel_path(root, rel);
    match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            cell(false, format!("cannot read {}: {error}", path.display()));
            Vec::new()
        }
    }
}

fn walk(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn into(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                into(root, &path, out);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.push((rel, fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    into(root, root, &mut out);
    out.sort();
    out
}

fn fresh_phone(parent: &Path) -> PairedRoot {
    let dir = parent.join("paired").join(ROOT_ID);
    fs::create_dir_all(&dir).unwrap();
    PairedRoot::new(dir)
}

/// Writes an unpacked pull under the phone's staging folder: every file of
/// `tree` plus the deck's progress document, listed by a pull manifest.
fn unpack_pull(
    phone: &PairedRoot,
    entry: &str,
    kind: &str,
    tree: &[(String, Vec<u8>)],
    deck_rel: &str,
    progress: &[u8],
) -> PathBuf {
    let dir = phone.staging().join(entry);
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    let document_rel = format!(".alix/progress/{DECK_ID}.json");
    let revision = serde_json::from_slice::<serde_json::Value>(progress).unwrap()["revision"]
        .as_u64()
        .expect("a progress document carries its revision");
    let mut files: Vec<(String, Vec<u8>)> = tree.to_vec();
    files.push((document_rel, progress.to_vec()));
    for (rel, bytes) in &files {
        let path = rel_path(&dir, rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    let manifest = SyncPullManifest {
        version: SYNC_PULL_MANIFEST_VERSION,
        root_id: ROOT_ID.to_string(),
        entry: entry.to_string(),
        kind: kind.to_string(),
        files: files
            .iter()
            .map(|(path, bytes)| SyncFileDto {
                path: path.clone(),
                bytes: bytes.len() as u64,
                fingerprint: fingerprint(bytes),
            })
            .collect(),
        decks: vec![SyncDeckDto {
            path: deck_rel.to_string(),
            deck_id: DECK_ID.to_string(),
            revision: Some(revision),
        }],
    };
    let manifest_path = dir.join(paired::MANIFEST_IN_ZIP);
    fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    dir
}

fn apply_pull(phone: &PairedRoot, unpacked: &Path) -> paired::PullReport {
    paired::apply_unpacked(phone, unpacked, &mut |_| Ok(())).unwrap()
}

/// The public projection a paired desktop serves for a workspace: the same
/// files a share with its sources stages, minus nothing.
fn staged_tree(
    path: &Path,
    parent: &Path,
    policy: SourcesPolicy,
) -> (PathBuf, Vec<(String, Vec<u8>)>) {
    let stage_root = parent.join(format!("stage-{}", parent.read_dir().unwrap().count()));
    fs::create_dir_all(&stage_root).unwrap();
    let staged = share::stage_path(path, &stage_root, policy).unwrap();
    let tree = walk(&staged.path)
        .into_iter()
        .filter(|(rel, _)| rel != ".alix-deck-share.json")
        .collect();
    (staged.path, tree)
}

#[test]
fn every_lifecycle_operation_carries_each_per_deck_folder_with_its_deck_and_never_the_roots_file() {
    let temp = TempDir::new().unwrap();
    let base = temp.path().canonicalize().unwrap();

    {
        let op = "deck copy";
        let source = workspace_at(&base, "copy-source");
        let destination = workspace_at(&base, "copy-destination");
        let member = sourced_member(&source);
        let roots = fs::read(rel_path(&source, &member.rel(Kind::Roots))).unwrap();
        let hidden = format!("sources/{DECK_ID}/.hidden");
        fs::write(rel_path(&source, &hidden), b"stays\n").unwrap();
        let report =
            deck_transfer::transfer(&deck_path(&source), &destination, TransferMode::Copy).unwrap();
        if report.sources != 2 {
            FAILED_CELLS.with(|cells| {
                cells.borrow_mut().push(format!(
                    "{op} / report.sources: {} source files counted, expected 2 (the roots file \
                     and a hidden file are not source files)",
                    report.sources
                ))
            });
        }
        check(
            op,
            "destination",
            &destination,
            &member,
            [true, true, true, true, true, false],
        );
        cell(
            fs::read(rel_path(&destination, &member.rel(Kind::Roots))).ok() == Some(roots),
            format!(
                "{op} / sources/<id>/manifest.local.toml / destination: the same machine's roots \
                 file comes along, byte for byte"
            ),
        );
        cell(
            !rel_path(&destination, &hidden).exists(),
            format!("{op} / sources/<id>/.hidden / destination: a hidden file stays home"),
        );
        check(op, "source", &source, &member, ALL);
    }

    {
        let op = "deck move";
        let source = workspace_at(&base, "move-source");
        let destination = workspace_at(&base, "move-destination");
        let member = sourced_member(&source);
        let roots = fs::read(rel_path(&source, &member.rel(Kind::Roots))).unwrap();
        deck_transfer::transfer(&deck_path(&source), &destination, TransferMode::Move).unwrap();
        check(op, "destination", &destination, &member, ALL);
        cell(
            fs::read(rel_path(&destination, &member.rel(Kind::Roots))).ok() == Some(roots),
            format!(
                "{op} / sources/<id>/manifest.local.toml / destination: the same machine's roots \
                 file moves along, byte for byte"
            ),
        );
        check(op, "source", &source, &member, NONE);
        cell(
            !source.join("sources").join(DECK_ID).exists(),
            format!("{op} / sources/<id> / source: the folder itself is gone"),
        );
    }

    {
        let op = "deck remove";
        let root = workspace_at(&base, "remove");
        let member = sourced_member(&root);
        let store = state::open_store(&deck_path(&root), &root).unwrap();
        library::remove_deck(&deck_path(&root), &store).unwrap();
        check(op, "source", &root, &member, NONE);
        cell(
            !root.join("sources").join(DECK_ID).exists(),
            format!("{op} / sources/<id> / source: the folder itself is gone"),
        );
    }

    {
        let op = "deck restore";
        let root = workspace_at(&base, "restore");
        let member = sourced_member(&root);
        let mut store = state::open_store(&deck_path(&root), &root).unwrap();
        library::replace_deck_retiring_backup_sources(
            &root.join("decks"),
            DECK_FILE,
            "---\nformat-version: 1\nid: \"deck-travel2\"\n---\n## new\nanswer\n",
            &mut store,
        )
        .unwrap();
        for kind in [Kind::Asset, Kind::Manifest, Kind::Copy, Kind::Roots] {
            cell(
                rel_path(&root, &member.rel(kind)).exists(),
                format!(
                    "{op} / {} / after a forced replace: kept while the replaced deck's backup exists",
                    kind.label()
                ),
            );
        }
        library::restore_deck(&deck_path(&root), &root).unwrap();
        check(op, "after restore", &root, &member, ALL);
        cell(
            DECK_TEXT.as_bytes() == read(&root, &format!("decks/{DECK_FILE}")).as_slice(),
            format!("{op}: the original deck is live again"),
        );
    }

    {
        let op = "workspace remove";
        let root = workspace_at(&base, "ws-remove");
        let member = sourced_member(&root);
        let store = state::open_store(&deck_path(&root), &root).unwrap();
        library::remove_workspace(&root, &store).unwrap();
        check(op, "source", &root, &member, NONE);
        cell(
            !root.join("sources").exists(),
            format!("{op} / sources / source: the workspace's sources folder is gone"),
        );
    }

    let shared = workspace_at(&base, "shared");
    let member = sourced_member(&shared);
    let desktop_roots = read(&shared, &member.rel(Kind::Roots));

    {
        let op = "share workspace (Omit)";
        let stage = base.join("share-ws-omit");
        fs::create_dir(&stage).unwrap();
        let staged = share::stage_path(&shared, &stage, SourcesPolicy::Omit).unwrap();
        check(op, "staged", &staged.path, &member, OMITTED);
        cell(
            2 == staged.sources_omitted,
            format!("{op}: the manifest and the copy were left out"),
        );
        check(op, "source", &shared, &member, ALL);

        let op = "receive workspace (Omit)";
        let inbox = base.join("inbox-ws-omit");
        fs::create_dir(&inbox).unwrap();
        let (name, stripped) = share::land_received(&stage, &inbox).unwrap();
        cell("shared" == name, format!("{op}: lands under its own name"));
        cell(
            stripped.is_empty(),
            format!("{op}: nothing private leaked: {stripped:?}"),
        );
        check(op, "landed", &inbox.join(&name), &member, OMITTED);
    }

    {
        let op = "share workspace (Carry)";
        let stage = base.join("share-ws-carry");
        fs::create_dir(&stage).unwrap();
        let staged = share::stage_path(&shared, &stage, SourcesPolicy::Carry).unwrap();
        check(op, "staged", &staged.path, &member, CARRIED);
        cell(
            0 == staged.sources_omitted,
            format!("{op}: nothing was left out"),
        );
        check(op, "source", &shared, &member, ALL);

        let op = "receive workspace (Carry)";
        let inbox = base.join("inbox-ws-carry");
        fs::create_dir(&inbox).unwrap();
        let (name, stripped) = share::land_received(&stage, &inbox).unwrap();
        cell(
            stripped.is_empty(),
            format!("{op}: nothing private leaked: {stripped:?}"),
        );
        check(op, "landed", &inbox.join(&name), &member, CARRIED);
        cell(
            read(&shared, &member.rel(Kind::Copy))
                == read(&inbox.join(&name), &member.rel(Kind::Copy)),
            format!("{op} / sources/<id>/<copy> / landed: byte-identical"),
        );
    }

    {
        let op = "share deck (Omit)";
        let stage = base.join("share-deck-omit");
        fs::create_dir(&stage).unwrap();
        let staged = share::stage_path(&deck_path(&shared), &stage, SourcesPolicy::Omit).unwrap();
        check(op, "staged", &staged.path, &member, OMITTED);
        cell(
            2 == staged.sources_omitted,
            format!("{op}: the manifest and the copy were left out"),
        );

        let op = "receive deck (Omit)";
        let receiver = workspace_at(&base, "receiver-deck-omit");
        let own_roots = rel_path(&receiver, &member.rel(Kind::Roots));
        fs::create_dir_all(own_roots.parent().unwrap()).unwrap();
        plant_roots(&own_roots);
        share::land_received(&stage, &receiver.join("decks")).unwrap();
        check(
            op,
            "landed",
            &receiver,
            &member,
            [true, true, false, false, true, false],
        );
        cell(
            fs::read(&own_roots).ok().as_deref() == Some(PHONE_ROOTS),
            format!(
                "{op} / sources/<id>/manifest.local.toml / landed: the receiver's own survives"
            ),
        );
    }

    {
        let op = "share deck (Carry)";
        let stage = base.join("share-deck-carry");
        fs::create_dir(&stage).unwrap();
        let staged = share::stage_path(&deck_path(&shared), &stage, SourcesPolicy::Carry).unwrap();
        check(op, "staged", &staged.path, &member, CARRIED);
        cell(
            0 == staged.sources_omitted,
            format!("{op}: nothing was left out"),
        );

        let op = "receive deck (Carry)";
        let receiver = workspace_at(&base, "receiver-deck-carry");
        let own_roots = rel_path(&receiver, &member.rel(Kind::Roots));
        fs::create_dir_all(own_roots.parent().unwrap()).unwrap();
        plant_roots(&own_roots);
        share::land_received(&stage, &receiver.join("decks")).unwrap();
        check(
            op,
            "landed",
            &receiver,
            &member,
            [true, true, true, true, true, false],
        );
        cell(
            fs::read(&own_roots).ok().as_deref() == Some(PHONE_ROOTS),
            format!(
                "{op} / sources/<id>/manifest.local.toml / landed: the receiver's own survives, not the sender's"
            ),
        );
        assert_ne!(
            desktop_roots,
            fs::read(&own_roots).unwrap(),
            "{op}: the sender's roots file never arrived"
        );
    }

    let progress = read(&shared, &member.rel(Kind::Progress));
    {
        let op = "paired pull workspace";
        let phone = fresh_phone(&base.join("phone-ws"));
        let (_, tree) = staged_tree(&shared, &base, SourcesPolicy::Carry);
        cell(
            tree.iter().all(|(rel, _)| !rel.ends_with(sources::ROOTS)),
            format!("{op} / sources/<id>/manifest.local.toml / projection: never listed: {tree:?}"),
        );
        let unpacked = unpack_pull(
            &phone,
            "shared",
            KIND_WORKSPACE,
            &tree,
            "decks/facts.md",
            &progress,
        );
        apply_pull(&phone, &unpacked);
        let entry_root = phone.entry_root(KIND_WORKSPACE, "shared");
        check(
            op,
            "first pull",
            &entry_root,
            &member,
            [true, true, true, true, false, true],
        );

        let own_roots = rel_path(&entry_root, &member.rel(Kind::Roots));
        plant_roots(&own_roots);
        let unpacked = unpack_pull(
            &phone,
            "shared",
            KIND_WORKSPACE,
            &tree,
            "decks/facts.md",
            &progress,
        );
        let report = apply_pull(&phone, &unpacked);
        check(op, "second pull", &entry_root, &member, ALL);
        cell(
            fs::read(&own_roots).ok().as_deref() == Some(PHONE_ROOTS),
            format!(
                "{op} / sources/<id>/manifest.local.toml / second pull: the phone's own survives the swap"
            ),
        );
        cell(
            report.phone_only.is_empty(),
            format!(
                "{op}: the phone's roots file is its own settings, not a phone-only deck file: {:?}",
                report.phone_only
            ),
        );
    }

    {
        let op = "paired pull deck";
        let phone = fresh_phone(&base.join("phone-deck"));
        let (_, tree) = staged_tree(&deck_path(&shared), &base, SourcesPolicy::Carry);
        let unpacked = unpack_pull(&phone, DECK_FILE, KIND_DECK, &tree, DECK_FILE, &progress);
        apply_pull(&phone, &unpacked);
        let entry_root = phone.entry_root(KIND_DECK, DECK_FILE);
        check(
            op,
            "first pull",
            &entry_root,
            &member,
            [true, true, true, true, false, true],
        );

        let own_roots = rel_path(&entry_root, &member.rel(Kind::Roots));
        plant_roots(&own_roots);
        let unpacked = unpack_pull(&phone, DECK_FILE, KIND_DECK, &tree, DECK_FILE, &progress);
        apply_pull(&phone, &unpacked);
        check(op, "second pull", &entry_root, &member, ALL);
        cell(
            fs::read(&own_roots).ok().as_deref() == Some(PHONE_ROOTS),
            format!(
                "{op} / sources/<id>/manifest.local.toml / second pull: the phone's own survives the folder swap"
            ),
        );

        let op = "paired remove entry";
        paired::remove_entry(&phone, DECK_FILE).unwrap();
        check(op, "phone", &entry_root, &member, NONE);
        cell(
            !entry_root.join("sources").join(DECK_ID).exists(),
            format!("{op} / sources/<id> / phone: the folder itself is gone"),
        );
    }

    {
        let op = "explore merge";
        let staging = base.join("merge-staging");
        let destination = base.join("merge-destination");
        fs::create_dir_all(staging.join("decks")).unwrap();
        fs::write(staging.join("alix.toml"), "title = \"Merged\"\n").unwrap();
        for kind in [Kind::Augment, Kind::Manifest, Kind::Copy] {
            let rel = member.rel(kind);
            let to = rel_path(&staging, &rel);
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            fs::write(to, read(&shared, &rel)).unwrap();
        }
        fs::write(staging.join("decks").join(DECK_FILE), DECK_TEXT).unwrap();
        assets::write_object(&staging, DECK_ID, b"excerpt\n", "md").unwrap();
        let mut store = Store::open(base.join("merge-store.json")).unwrap();
        let report = explore::merge_built(&staging, &destination, false, &mut store).unwrap();
        cell(
            report.conflicts.is_empty(),
            format!("{op}: a clean merge: {:?}", report.conflicts),
        );
        check(op, "destination", &destination, &member, CARRIED);
        cell(
            !staging.join("sources").join(DECK_ID).exists(),
            format!("{op} / sources/<id> / staging: moved, not copied"),
        );

        let op = "explore merge (conflict)";
        for kind in [Kind::Manifest, Kind::Copy] {
            let rel = member.rel(kind);
            let to = rel_path(&staging, &rel);
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            fs::write(to, read(&shared, &rel)).unwrap();
        }
        let own_roots = rel_path(&destination, &member.rel(Kind::Roots));
        plant_roots(&own_roots);
        let report = explore::merge_built(&staging, &destination, false, &mut store).unwrap();
        cell(
            vec![format!("sources/{DECK_ID}")] == report.conflicts,
            format!("{op}: an existing sources folder is one conflict"),
        );
        cell(
            staging.join("sources").join(DECK_ID).is_dir(),
            format!("{op} / sources/<id> / staging: kept for the user to decide"),
        );

        let op = "explore merge (force)";
        let report = explore::merge_built(&staging, &destination, true, &mut store).unwrap();
        cell(
            report.conflicts.is_empty(),
            format!("{op}: forced: {:?}", report.conflicts),
        );
        check(
            op,
            "destination",
            &destination,
            &member,
            [true, true, true, true, true, false],
        );
        cell(
            fs::read(&own_roots).ok().as_deref() == Some(PHONE_ROOTS),
            format!(
                "{op} / sources/<id>/manifest.local.toml / destination: the destination's own survives the replacement"
            ),
        );
    }

    #[cfg(unix)]
    {
        let op = "workspace update publish";
        let root = base.join("update");
        let (unfrozen_text, member) = frozen_member_for_update(&root);
        let command = fake_backend(&base.join("update-backend"), &unfrozen_text);
        let ask = alix::config::AskConfig {
            command: command.to_str().unwrap().to_string(),
            timeout_secs: 10,
            ..alix::config::AskConfig::default()
        };
        let own_roots = rel_path(&root, &member.rel(Kind::Roots));
        let roots_before = fs::read(&own_roots).unwrap();
        let staged = alix::workspace_update::stage(
            &root,
            &alix::config::GenerateDeckConfig::default(),
            &ask,
        )
        .unwrap();
        check(
            op,
            "staging",
            &staged.staging,
            &member,
            [true, true, true, true, true, false],
        );
        fs::remove_file(rel_path(&root, &member.rel(Kind::Copy))).unwrap();
        alix::workspace_update::apply(&root).unwrap();
        check(op, "published", &root, &member, ALL);
        cell(
            SOURCE_BYTES == read(&root, &member.rel(Kind::Copy)).as_slice(),
            format!("{op} / sources/<id>/<copy> / published: the copy is back with its bytes"),
        );
        cell(
            Some(&roots_before) == fs::read(&own_roots).ok().as_ref(),
            format!("{op} / sources/<id>/manifest.local.toml / published: untouched"),
        );
    }
    assert_every_cell_held();
}

/// A frozen, source-backed member the way `alix workspace update` wants one:
/// `cite:` points at a folder inside the workspace, the card cites
/// it, and one frozen source is bound to the deck's leading topic. Returns the
/// deck text before freezing, which is also an unchanged proposal for it.
#[cfg(unix)]
fn frozen_member_for_update(root: &Path) -> (String, Member) {
    fs::create_dir_all(root.join("decks")).unwrap();
    fs::write(root.join("alix.toml"), "").unwrap();
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("code.rs"), "Old answer\n").unwrap();
    let deck = root.join("decks").join(DECK_FILE);
    let unfrozen_text = format!(
        "---\nformat-version: 1\nid: \"{DECK_ID}\"\ncite:\n  - src: {0}\n---\n## Old \
         question\nOld answer\n<!-- at: src:code.rs:1 -->\n<!-- id: {CARD_ID} -->\n",
        alix::parser::yaml_quote(&source.display().to_string())
    );
    fs::write(&deck, &unfrozen_text).unwrap();
    assets::freeze_member(&deck).unwrap();
    let loaded = Deck::load(&deck).unwrap();
    let asset = loaded.cards[0]
        .citations
        .iter()
        .find_map(|citation| citation.asset.clone())
        .expect("the frozen citation names its asset");
    let mut cache = AugmentCache::open_for_deck(&loaded).unwrap();
    cache.set_note(
        CARD_ID,
        "note".to_string(),
        loaded.cards[0].content_fingerprint,
    );
    cache.save().unwrap();
    let mut store = state::open_store(&deck, root).unwrap();
    store.get_or_insert(CARD_ID);
    store.save().unwrap();
    let live = root.join("live");
    fs::create_dir_all(&live).unwrap();
    let file = live.join("notes.txt");
    fs::write(&file, SOURCE_BYTES).unwrap();
    let report = commands::add(
        AddRequest {
            deck: &deck,
            files: vec![file],
            root: Some(("sdk".to_string(), live)),
            key: None,
            topics: Vec::new(),
        },
        &ExamConfig::default(),
    )
    .unwrap();
    let member = Member {
        asset,
        copy: format!("{}.txt", report.added[0].0),
    };
    assert_fixture_whole("update fixture", root, &member);
    (unfrozen_text, member)
}

#[cfg(unix)]
fn fake_backend(dir: &Path, reply: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(dir).unwrap();
    let out = dir.join("reply");
    fs::write(&out, reply).unwrap();
    let path = dir.join("fake-claude");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nPATH=/usr/bin:/bin\ncat >/dev/null\ncat {}\n",
            out.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
fn fake_ask(dir: &Path, reply: &str) -> alix::config::AskConfig {
    alix::config::AskConfig {
        command: fake_backend(dir, reply).to_str().unwrap().to_string(),
        timeout_secs: 10,
        ..alix::config::AskConfig::default()
    }
}

#[test]
fn paired_sync_both_directions_keeps_each_sides_roots_file_and_crosses_neither() {
    let temp = TempDir::new().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let desktop = workspace_at(&base, "desktop");
    let member = sourced_member(&desktop);
    let desktop_roots = read(&desktop, &member.rel(Kind::Roots));
    assert!(
        desktop_roots.windows(3).any(|window| window == b"sdk"),
        "the desktop's roots file names its key: {}",
        String::from_utf8_lossy(&desktop_roots)
    );
    let progress = read(&desktop, &member.rel(Kind::Progress));
    let phone = fresh_phone(&base.join("phone"));
    let (_, tree) = staged_tree(&desktop, &base, SourcesPolicy::Carry);

    let unpacked = unpack_pull(
        &phone,
        "desktop",
        KIND_WORKSPACE,
        &tree,
        "decks/facts.md",
        &progress,
    );
    apply_pull(&phone, &unpacked);
    let entry_root = phone.entry_root(KIND_WORKSPACE, "desktop");
    let phone_roots = rel_path(&entry_root, &member.rel(Kind::Roots));
    assert!(
        !phone_roots.exists(),
        "desktop -> phone: the desktop's roots file never arrives"
    );
    plant_roots(&phone_roots);

    let unpacked = unpack_pull(
        &phone,
        "desktop",
        KIND_WORKSPACE,
        &tree,
        "decks/facts.md",
        &progress,
    );
    apply_pull(&phone, &unpacked);
    assert_eq!(
        PHONE_ROOTS,
        fs::read(&phone_roots).unwrap().as_slice(),
        "desktop -> phone, second pull: the phone keeps its own roots file"
    );
    assert_eq!(
        desktop_roots,
        read(&desktop, &member.rel(Kind::Roots)),
        "phone -> desktop: the desktop keeps its own roots file"
    );
    for (rel, bytes) in walk(&entry_root) {
        assert_ne!(
            desktop_roots, bytes,
            "desktop -> phone: the desktop's roots bytes are nowhere on the phone ({rel})"
        );
    }

    let mut smuggled = tree.clone();
    smuggled.push((member.rel(Kind::Roots), desktop_roots.clone()));
    let unpacked = unpack_pull(
        &phone,
        "desktop",
        KIND_WORKSPACE,
        &smuggled,
        "decks/facts.md",
        &progress,
    );
    let error = paired::apply_unpacked(&phone, &unpacked, &mut |_| Ok(())).unwrap_err();
    assert!(
        format!("{error:#}").contains(sources::ROOTS),
        "a pull listing a roots file is refused by name: {error:#}"
    );
    assert_eq!(
        PHONE_ROOTS,
        fs::read(&phone_roots).unwrap().as_slice(),
        "a refused pull leaves the phone's roots file alone"
    );
}

type Fault<'a> = (&'a str, &'a dyn Fn(&Path));

#[test]
fn a_landing_refuses_a_tampered_or_linked_source_copy_before_writing_the_deck() {
    let temp = TempDir::new().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let sender = workspace_at(&base, "sender");
    let member = sourced_member(&sender);
    let copy = member.rel(Kind::Copy);

    let tamper = |bundle: &Path| fs::write(rel_path(bundle, &copy), b"changed\n").unwrap();
    #[cfg(unix)]
    let link = |bundle: &Path| {
        let path = rel_path(bundle, &copy);
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(sender.join("live").join("notes.txt"), path).unwrap();
    };

    #[cfg(unix)]
    let faults: Vec<Fault> = vec![("tampered", &tamper), ("linked", &link)];
    #[cfg(not(unix))]
    let faults: Vec<Fault> = vec![("tampered", &tamper)];

    for (fault, apply_fault) in faults {
        let stage = base.join(format!("deck-{fault}"));
        fs::create_dir(&stage).unwrap();
        let staged = share::stage_path(&deck_path(&sender), &stage, SourcesPolicy::Carry).unwrap();
        apply_fault(&staged.path);
        let receiver = workspace_at(&base, &format!("receiver-deck-{fault}"));
        let error = share::land_deck_bundle(&staged.path, &receiver.join("decks")).unwrap_err();
        assert!(
            format!("{error:#}").contains("source"),
            "deck bundle, {fault}: the refusal names the source copy: {error:#}"
        );
        assert!(
            !deck_path(&receiver).exists(),
            "deck bundle, {fault}: the deck was not written"
        );
        assert!(
            !receiver.join("sources").join(DECK_ID).exists(),
            "deck bundle, {fault}: no sources folder was created"
        );
        assert!(
            !receiver.join("assets").join(DECK_ID).exists(),
            "deck bundle, {fault}: no assets folder was created"
        );

        let stage = base.join(format!("ws-{fault}"));
        fs::create_dir(&stage).unwrap();
        let staged = share::stage_path(&sender, &stage, SourcesPolicy::Carry).unwrap();
        apply_fault(&staged.path);
        let inbox = base.join(format!("inbox-{fault}"));
        fs::create_dir(&inbox).unwrap();
        let error = share::land_received(&stage, &inbox).unwrap_err();
        assert!(
            format!("{error:#}").contains("source"),
            "workspace, {fault}: the refusal names the source copy: {error:#}"
        );
        assert!(
            !inbox.join("sender").exists(),
            "workspace, {fault}: nothing landed"
        );

        // A tampered copy is listed with its own fingerprint, so only the source
        // check can catch it; a link cannot be zipped past `unpack`, so it is
        // planted in the unpacked tree, where the manifest fingerprint still
        // matches the bytes behind it.
        let phone = fresh_phone(&base.join(format!("phone-{fault}")));
        let (staged, _) = staged_tree(&sender, &base, SourcesPolicy::Carry);
        if fault == "tampered" {
            apply_fault(&staged);
        }
        let tree = walk(&staged);
        let progress = read(&sender, &member.rel(Kind::Progress));
        let unpacked = unpack_pull(
            &phone,
            "sender",
            KIND_WORKSPACE,
            &tree,
            "decks/facts.md",
            &progress,
        );
        if fault == "linked" {
            apply_fault(&unpacked);
        }
        let error = paired::apply_unpacked(&phone, &unpacked, &mut |_| Ok(())).unwrap_err();
        assert!(
            format!("{error:#}").contains("source"),
            "paired pull, {fault}: the refusal names the source copy: {error:#}"
        );
        assert!(
            !phone.entry_root(KIND_WORKSPACE, "sender").exists(),
            "paired pull, {fault}: nothing landed"
        );
        assert!(
            !phone.manifest_path("sender").exists(),
            "paired pull, {fault}: no manifest was recorded"
        );
    }
}

#[test]
fn public_share_without_the_flag_omits_sources_and_says_so_and_with_it_carries_all() {
    let temp = TempDir::new().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let home = base.join("home");
    fs::create_dir_all(&home).unwrap();
    let shared = workspace_at(&base, "shared");
    let member = sourced_member(&shared);
    let rows: [(&str, PathBuf, &str); 2] = [
        ("workspace", shared.clone(), "shared"),
        ("deck", deck_path(&shared), "facts.alix-deck"),
    ];
    for (what, path, landed) in rows {
        for with_sources in [false, true] {
            let out_dir = base.join(format!("out-{what}-{with_sources}"));
            fs::create_dir(&out_dir).unwrap();
            let mut args = vec![
                "share".to_string(),
                path.to_str().unwrap().to_string(),
                "--zip".to_string(),
                "--output".to_string(),
                out_dir.to_str().unwrap().to_string(),
            ];
            if with_sources {
                args.push("--with-sources".to_string());
            }
            let output = Command::new(env!("CARGO_BIN_EXE_alix"))
                .args(&args)
                .env("HOME", &home)
                .env("XDG_CONFIG_HOME", &home)
                .env("XDG_DATA_HOME", &home)
                .env("XDG_STATE_HOME", &home)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success(),
                "{what}, with_sources={with_sources}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let said =
                stdout.contains("left out 2 source files; the receiver cannot take the exam");
            assert_eq!(
                !with_sources, said,
                "{what}, with_sources={with_sources}: the omission is said exactly when it happens: {stdout}"
            );
            let zip = fs::read_dir(&out_dir)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| path.extension().is_some_and(|ext| ext == "zip"))
                .expect("one archive");
            let landed_dir = base.join(format!("landed-{what}-{with_sources}"));
            share::unzip_to(&zip, &landed_dir).unwrap();
            let expected = if with_sources { CARRIED } else { OMITTED };
            check(
                &format!("alix share --zip {what} with_sources={with_sources}"),
                "archive",
                &landed_dir.join(landed),
                &member,
                expected,
            );
        }
    }
    assert_every_cell_held();
}

#[test]
fn the_asset_collector_reads_only_the_assets_directory_never_a_source_folder() {
    let temp = TempDir::new().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let root = workspace_at(&base, "collector");
    let member = sourced_member(&root);
    let lookalike = root
        .join("sources")
        .join(DECK_ID)
        .join(format!("asset-{}.md", "0".repeat(52)));
    fs::write(&lookalike, b"not an asset\n").unwrap();
    let deck = Deck::load(deck_path(&root)).unwrap();
    let orphans = assets::unreferenced_objects(&deck, &root).unwrap();
    assert_eq!(
        vec![member.asset.clone()],
        orphans,
        "the collector lists the unreferenced asset object and nothing from sources/, not even \
         an object-named file there"
    );
    assert!(
        lookalike.is_file(),
        "the object-named file under sources/ is still there to be missed"
    );
    assert!(
        root.join("sources")
            .join(DECK_ID)
            .join(&member.copy)
            .is_file(),
        "the source copy is still there to be missed"
    );
}

#[test]
fn copying_a_deck_whose_topic_id_the_destination_uses_is_refused() {
    let temp = TempDir::new().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let source = workspace_at(&base, "source");
    sourced_member(&source);
    let destination = workspace_at(&base, "destination");
    let member = destination.join("decks").join("other.md");
    fs::write(
        &member,
        "---\nformat-version: 1\nid: \"deck-other1\"\n---\n# Alpha\n<!-- id: topic-alpha1 -->\n\n## \
         other\nanswer\n<!-- id: card-o1 -->\n",
    )
    .unwrap();
    let before = walk(&destination);

    let error =
        deck_transfer::transfer(&deck_path(&source), &destination, TransferMode::Copy).unwrap_err();

    let text = format!("{error:#}");
    assert!(
        text.contains(&format!(
            "topic id `{TOPIC_ID}` is also at {}:6",
            member.display()
        )),
        "the refusal names the member holding the topic id as the keeper: {text}"
    );
    assert!(
        text.contains(&format!(
            "delete the id line at {}:6",
            deck_path(&source).display()
        )),
        "the refusal names the moving deck's id line as the copy: {text}"
    );
    assert_eq!(before, walk(&destination), "the destination is unchanged");
}

#[test]
fn a_same_machine_copy_and_move_carry_the_roots_file_into_the_destination() {
    let temp = TempDir::new().unwrap();
    let base = temp.path().canonicalize().unwrap();
    for (mode, name) in [(TransferMode::Copy, "copy"), (TransferMode::Move, "move")] {
        let source = workspace_at(&base, &format!("{name}-source"));
        let destination = workspace_at(&base, &format!("{name}-destination"));
        let member = sourced_member(&source);
        let roots_rel = member.rel(Kind::Roots);
        let roots = fs::read(rel_path(&source, &roots_rel)).unwrap();
        assert!(
            String::from_utf8_lossy(&roots).contains("sdk"),
            "{name}: the fixture's roots file names its key"
        );

        deck_transfer::transfer(&deck_path(&source), &destination, mode).unwrap();

        assert_eq!(
            Some(roots.as_slice()),
            fs::read(rel_path(&destination, &roots_rel)).ok().as_deref(),
            "{name}: the destination holds the author's roots file, byte for byte"
        );
        assert_eq!(
            mode == TransferMode::Copy,
            rel_path(&source, &roots_rel).is_file(),
            "{name}: the source keeps its roots file only on copy"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_workspace_update_refuses_to_apply_after_a_source_command_changed_the_manifest() {
    let temp = TempDir::new().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let root = base.join("update");
    let (unfrozen_text, member) = frozen_member_for_update(&root);
    let ask = fake_ask(&base.join("backend"), &unfrozen_text);
    let staged =
        alix::workspace_update::stage(&root, &alix::config::GenerateDeckConfig::default(), &ask)
            .unwrap();

    let more = root.join("live").join("more.txt");
    fs::write(&more, b"a second source\n").unwrap();
    let report = commands::add(
        AddRequest {
            deck: &deck_path(&root),
            files: vec![more],
            root: None,
            key: Some("sdk".to_string()),
            topics: Vec::new(),
        },
        &ExamConfig::default(),
    )
    .unwrap();
    let manifest_rel = member.rel(Kind::Manifest);
    let manifest_before = fs::read(rel_path(&root, &manifest_rel)).unwrap();
    assert!(
        String::from_utf8_lossy(&manifest_before).contains(&report.added[0].0),
        "the live manifest names the source added after staging"
    );

    let error = alix::workspace_update::apply(&root).unwrap_err();

    assert!(
        format!("{error:#}").contains("changed after the proposal was staged"),
        "apply refuses the stale proposal: {error:#}"
    );
    assert_eq!(
        manifest_before,
        fs::read(rel_path(&root, &manifest_rel)).unwrap(),
        "the live manifest keeps both rows"
    );
    assert!(
        staged.staging.is_dir(),
        "the proposal stays for discard or regenerate"
    );
}

#[cfg(unix)]
#[test]
fn a_workspace_update_refuses_a_tampered_staged_copy_before_publishing_anything() {
    let temp = TempDir::new().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let root = base.join("update");
    let (unfrozen_text, member) = frozen_member_for_update(&root);
    let ask = fake_ask(&base.join("backend"), &unfrozen_text);
    let staged =
        alix::workspace_update::stage(&root, &alix::config::GenerateDeckConfig::default(), &ask)
            .unwrap();
    let copy_rel = member.rel(Kind::Copy);
    fs::write(rel_path(&staged.staging, &copy_rel), b"changed\n").unwrap();
    let planted = assets::write_object(&staged.staging, DECK_ID, b"planted\n", "md").unwrap();
    let planted_name = planted.file_name().unwrap().to_os_string();
    let deck_before = fs::read(deck_path(&root)).unwrap();

    let error = alix::workspace_update::apply(&root).unwrap_err();

    assert!(
        format!("{error:#}").contains("source"),
        "the refusal names the source copy: {error:#}"
    );
    assert!(
        !root
            .join("assets")
            .join(DECK_ID)
            .join(&planted_name)
            .exists(),
        "the refusal comes before any publishing: the asset planted in staging never went live"
    );
    assert_eq!(
        SOURCE_BYTES,
        fs::read(rel_path(&root, &copy_rel)).unwrap().as_slice(),
        "the live copy is intact"
    );
    assert_eq!(
        deck_before,
        fs::read(deck_path(&root)).unwrap(),
        "the live deck is intact"
    );
}

#[test]
fn an_explore_merge_refuses_a_tampered_source_copy_in_the_staging() {
    let temp = TempDir::new().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let built = workspace_at(&base, "built");
    let member = sourced_member(&built);
    let (staging, _) = staged_tree(&built, &base, SourcesPolicy::Carry);
    let copy_rel = member.rel(Kind::Copy);
    fs::write(rel_path(&staging, &copy_rel), b"changed\n").unwrap();
    let destination = base.join("merge-destination");
    let mut store = Store::open(base.join("merge-store.json")).unwrap();

    let error = explore::merge_built(&staging, &destination, false, &mut store).unwrap_err();

    assert!(
        format!("{error:#}").contains("source"),
        "the refusal names the source copy: {error:#}"
    );
    assert!(
        !destination.join("sources").join(DECK_ID).exists(),
        "the tampered folder did not land"
    );
    assert!(
        !destination.join("decks").exists(),
        "sources are verified before any deck lands"
    );
    assert!(
        rel_path(&staging, &copy_rel).is_file(),
        "the staging keeps the folder for the user"
    );
}

#[test]
fn paired_recover_restores_a_sources_folder_a_crash_left_aside() {
    let temp = TempDir::new().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let desktop = workspace_at(&base, "desktop");
    let member = sourced_member(&desktop);
    let progress = fs::read(rel_path(&desktop, &member.rel(Kind::Progress))).unwrap();
    let phone = fresh_phone(&base.join("phone"));
    let (_, tree) = staged_tree(&deck_path(&desktop), &base, SourcesPolicy::Carry);
    let unpacked = unpack_pull(&phone, DECK_FILE, KIND_DECK, &tree, DECK_FILE, &progress);
    apply_pull(&phone, &unpacked);
    let entry_root = phone.entry_root(KIND_DECK, DECK_FILE);
    let live = entry_root.join("sources").join(DECK_ID);
    let aside = entry_root.join("sources").join(format!("{DECK_ID}.old"));
    fs::rename(&live, &aside).unwrap();

    let actions = paired::recover(&phone).unwrap();

    assert!(
        actions
            .iter()
            .any(|action| action == &format!("restored {DECK_ID}")),
        "recover names the restored sources folder: {actions:?}"
    );
    assert!(
        live.join(sources::MANIFEST).is_file() && !aside.exists(),
        "the folder is back under its live name"
    );
}

#[test]
fn tidy_renamed_tidies_an_entry_whose_only_extra_file_is_the_phones_roots_file() {
    let temp = TempDir::new().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let desktop = workspace_at(&base, "desktop");
    let member = sourced_member(&desktop);
    let progress = fs::read(rel_path(&desktop, &member.rel(Kind::Progress))).unwrap();
    let phone = fresh_phone(&base.join("phone"));
    let (_, tree) = staged_tree(&desktop, &base, SourcesPolicy::Carry);
    let pull = |entry: &str| {
        let unpacked = unpack_pull(
            &phone,
            entry,
            KIND_WORKSPACE,
            &tree,
            "decks/facts.md",
            &progress,
        );
        apply_pull(&phone, &unpacked);
    };
    pull("desktop");
    let old_root = phone.entry_root(KIND_WORKSPACE, "desktop");
    let own_roots = rel_path(&old_root, &member.rel(Kind::Roots));
    plant_roots(&own_roots);
    pull("desktop");
    assert!(
        own_roots.is_file(),
        "the phone's roots file survived the second pull"
    );
    pull("renamed");

    let renamed = paired::tidy_renamed(&phone, &["renamed".to_string()]).unwrap();

    assert_eq!(
        vec![("desktop".to_string(), "renamed".to_string())],
        renamed,
        "the roots file is the phone's settings, not a phone-born member that keeps the old entry"
    );
    assert!(!old_root.exists(), "the renamed entry's old folder is gone");
}
