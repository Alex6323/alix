use std::{
    fs,
    io::{Seek, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bundle {
    pub path: PathBuf,
    pub files: usize,
    pub bytes: u64,
}

pub struct BundleOptions<'a> {
    pub root: &'a Path,
    pub out_dir: &'a Path,
    pub config_path: Option<&'a Path>,
    pub log_paths: &'a [PathBuf],
    pub include_deck: Option<&'a Path>,
    pub home: &'a Path,
    pub tokens: &'a [String],
    pub now_ms: u64,
}

pub fn write_bundle(root: &Path, out_dir: &Path, now_ms: u64) -> Result<Bundle> {
    let base = directories::BaseDirs::new().context("cannot determine the home directory")?;
    let config_path = crate::config::default_config_path().filter(|path| path.is_file());
    let log_paths = crate::log::log_paths()?;
    write_bundle_with(&BundleOptions {
        root,
        out_dir,
        config_path: config_path.as_deref(),
        log_paths: &log_paths,
        include_deck: None,
        home: base.home_dir(),
        tokens: &[],
        now_ms,
    })
}

pub fn write_bundle_with(options: &BundleOptions<'_>) -> Result<Bundle> {
    if !options.root.is_dir() {
        bail!("{} is not a decks folder", options.root.display());
    }
    let included_deck = options.include_deck.map(read_included_deck).transpose()?;
    fs::create_dir_all(options.out_dir)
        .with_context(|| format!("cannot create {}", options.out_dir.display()))?;
    let timestamp = timestamp(options.now_ms)?;
    let filename = format!("alix-bug-report-{}-{timestamp}.zip", crate::VERSION);
    let output = options.out_dir.join(filename);
    if output.exists() {
        bail!("{} already exists", output.display());
    }

    let stage = tempfile::Builder::new()
        .prefix(".alix-bug-report-")
        .tempdir_in(options.out_dir)
        .with_context(|| {
            format!(
                "cannot stage the bug report in {}",
                options.out_dir.display()
            )
        })?;
    let mut tokens = options.tokens.to_vec();
    let safe_config = options
        .config_path
        .and_then(|path| read_safe_config(path, &mut tokens).transpose())
        .transpose()?;

    let included_logs = copy_logs(
        options.log_paths,
        &stage.path().join("log"),
        options.home,
        &tokens,
    )?;
    if let Some(deck) = &included_deck {
        fs::write(stage.path().join("deck.md"), &deck.bytes)
            .context("cannot stage the included deck")?;
    }
    let deck_inventory = deck_inventory(options.root);
    let report = format!(
        "# Alix bug report\n\nGenerated: {timestamp}\nAlix: {}\nBuild: {}\nOS: {}\nArchitecture: {}\nDecks: {}\nLogs: {}\nIncluded deck: {}\n\nThis archive was written locally and was not sent anywhere. Review every file before attaching it to a bug report. In particular, inspect the diagnostic text under `log/` and the verbatim `deck.md` when present.\n",
        crate::VERSION,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        std::env::consts::OS,
        std::env::consts::ARCH,
        deck_inventory.len(),
        if included_logs.is_empty() {
            "none".to_string()
        } else {
            included_logs.join(", ")
        },
        included_deck.as_ref().map_or("none", |deck| &deck.name),
    );
    write_redacted(
        &stage.path().join("report.md"),
        &report,
        options.home,
        &tokens,
    )?;

    if let Some(config) = safe_config {
        write_redacted(
            &stage.path().join("config.toml"),
            &config,
            options.home,
            &tokens,
        )?;
    }

    let decks = deck_inventory
        .iter()
        .map(|row| {
            format!(
                "deck_sha256={} cards={} progress={} reviews={}",
                row.deck_hash, row.cards, row.progress, row.reviews
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    write_redacted(
        &stage.path().join("decks.txt"),
        &format!("{decks}\n"),
        options.home,
        &tokens,
    )?;

    let mut temporary = tempfile::NamedTempFile::new_in(options.out_dir)
        .with_context(|| format!("cannot stage the archive in {}", options.out_dir.display()))?;
    let files = zip_contents(stage.path(), temporary.as_file_mut())?;
    temporary
        .persist_noclobber(&output)
        .map_err(|error| error.error)
        .with_context(|| format!("cannot publish {}", output.display()))?;
    let bytes = fs::metadata(&output)
        .with_context(|| format!("cannot inspect {}", output.display()))?
        .len();
    Ok(Bundle {
        path: output,
        files,
        bytes,
    })
}

struct IncludedDeck {
    name: String,
    bytes: Vec<u8>,
}

fn read_included_deck(path: &Path) -> Result<IncludedDeck> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("the included deck must have a portable file name")?;
    if crate::workspace::is_sidecar_name(name) {
        bail!("cannot include a personal sidecar in a bug report");
    }
    if path.extension().is_none_or(|extension| extension != "md") {
        bail!("{} is not a Markdown deck", path.display());
    }
    let resolved = fs::canonicalize(path)
        .with_context(|| format!("cannot read the included deck {}", path.display()))?;
    if resolved
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(crate::workspace::is_sidecar_name)
    {
        bail!("cannot include a personal sidecar in a bug report");
    }
    let metadata = fs::metadata(&resolved)
        .with_context(|| format!("cannot inspect the included deck {}", path.display()))?;
    if !metadata.is_file() {
        bail!("{} is not a deck file", path.display());
    }
    let bytes = fs::read(&resolved)
        .with_context(|| format!("cannot read the included deck {}", path.display()))?;
    Ok(IncludedDeck {
        name: name.to_string(),
        bytes,
    })
}

pub fn redact(text: &str, home: &Path, tokens: &[String]) -> String {
    let mut output = text.to_string();
    let mut tokens = tokens
        .iter()
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    tokens.sort_by_key(|token| std::cmp::Reverse(token.len()));
    tokens.dedup();
    for token in tokens {
        output = output.replace(token, "<redacted>");
    }
    if let Some(home) = home.to_str().filter(|home| !home.is_empty()) {
        output = output.replace(home, "~");
    }
    if let Some(username) = home.file_name().and_then(|name| name.to_str())
        && !username.is_empty()
    {
        output = output.replace(username, "~");
    }
    output
}

fn timestamp(now_ms: u64) -> Result<String> {
    let millis = i64::try_from(now_ms).context("the report time is out of range")?;
    let instant = chrono::DateTime::from_timestamp_millis(millis)
        .context("the report time is out of range")?;
    Ok(instant.format("%Y%m%dT%H%M%SZ").to_string())
}

fn read_safe_config(path: &Path, tokens: &mut Vec<String>) -> Result<Option<String>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("cannot read {}", path.display()));
        }
    };
    let mut value: toml::Value =
        toml::from_str(&text).with_context(|| format!("cannot safely read {}", path.display()))?;
    drop_private_config_fields(&mut value, tokens);
    Ok(Some(toml::to_string_pretty(&value)?))
}

fn drop_private_config_fields(value: &mut toml::Value, tokens: &mut Vec<String>) {
    match value {
        toml::Value::Table(table) => {
            if let Some(value) = table.remove("token")
                && let toml::Value::String(token) = value
                && !token.is_empty()
            {
                tokens.push(token);
            }
            table.remove("prompt");
            table.remove("extra");
            for (_, value) in table.iter_mut() {
                drop_private_config_fields(value, tokens);
            }
        }
        toml::Value::Array(values) => {
            for value in values {
                drop_private_config_fields(value, tokens);
            }
        }
        _ => {}
    }
}

fn write_redacted(path: &Path, text: &str, home: &Path, tokens: &[String]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    fs::write(path, redact(text, home, tokens))
        .with_context(|| format!("cannot write {}", path.display()))
}

fn copy_logs(
    sources: &[PathBuf],
    destination: &Path,
    home: &Path,
    tokens: &[String],
) -> Result<Vec<String>> {
    let mut included = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let current = if index == 0 {
            "alix.log".to_string()
        } else {
            format!("alix-{}.log", index + 1)
        };
        if copy_log(source, &destination.join(&current), home, tokens)? {
            included.push(format!("log/{current}"));
        }
        let rollover = format!("{current}.1");
        if copy_log(
            &source.with_extension("log.1"),
            &destination.join(&rollover),
            home,
            tokens,
        )? {
            included.push(format!("log/{rollover}"));
        }
    }
    Ok(included)
}

fn copy_log(from: &Path, to: &Path, home: &Path, tokens: &[String]) -> Result<bool> {
    let bytes = match fs::read(from) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("cannot read {}", from.display()));
        }
    };
    let safe = allowlisted_log(&String::from_utf8_lossy(&bytes));
    write_redacted(to, &safe, home, tokens)?;
    Ok(true)
}

fn allowlisted_log(text: &str) -> String {
    let mut output = String::new();
    for line in text.lines() {
        let fields = line
            .split_ascii_whitespace()
            .filter_map(|field| field.split_once('='))
            .collect::<Vec<_>>();
        let Some(target) = fields
            .iter()
            .find_map(|(key, value)| (*key == "target").then_some(*value))
        else {
            continue;
        };
        let allowed: &[&str] = match target {
            "http" => &["target", "at", "took", "w"],
            "select" => &[
                "target", "card", "tier", "fresh", "revealed", "due", "floor", "roster",
            ],
            "error" => match fields
                .iter()
                .find_map(|(key, value)| (*key == "kind").then_some(*value))
            {
                Some("ai") => &["target", "kind", "backend"],
                Some("http") => &["target", "kind", "method", "area", "status"],
                Some("panic") => &["target", "kind", "file", "line", "thread"],
                Some("parse") => &["target", "kind", "code", "line"],
                _ => continue,
            },
            _ => continue,
        };
        let safe = fields
            .into_iter()
            .filter(|(key, _)| allowed.contains(key))
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>();
        if !safe.is_empty() {
            output.push_str(&safe.join(" "));
            output.push('\n');
        }
    }
    output
}

struct DeckInventory {
    deck_hash: String,
    cards: usize,
    progress: usize,
    reviews: u32,
}

fn deck_inventory(root: &Path) -> Vec<DeckInventory> {
    let mut paths = Vec::new();
    collect_decks(root, &mut paths);
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| inventory_row(root, &path))
        .collect()
}

fn collect_decks(dir: &Path, paths: &mut Vec<PathBuf>) {
    let mut entries = match fs::read_dir(dir) {
        Ok(entries) => entries.flatten().collect::<Vec<_>>(),
        Err(_) => return,
    };
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if !matches!(name.as_ref(), "progress" | "augment" | "assets") {
                collect_decks(&path, paths);
            }
        } else if kind.is_file()
            && path.extension().is_some_and(|extension| extension == "md")
            && !crate::workspace::is_conventional_non_deck(&name)
            && !crate::workspace::is_conflict_name(&name)
            && !crate::workspace::is_sidecar_name(&name)
            && crate::workspace::file_is_deck(&path)
        {
            paths.push(path);
        }
    }
}

fn inventory_row(root: &Path, path: &Path) -> Option<DeckInventory> {
    let deck = crate::deck::Deck::load(path).ok()?;
    let deck_id = deck.deck_token.as_deref()?;
    let deck_hash = hex_digest(deck_id.as_bytes());
    let user_root = crate::workspace::root_for_deck(path)
        .map(|workspace| crate::workspace::store_path(&workspace))
        .unwrap_or_else(|| crate::workspace::root_store_path(root));
    let progress_path = crate::state::UserFiles::new(user_root).progress_for(deck_id);
    let (progress, reviews) =
        match crate::store::Store::open_deck(&progress_path, deck_id, &deck.subject) {
            Ok(store) => {
                let reviews = deck
                    .cards
                    .iter()
                    .filter_map(|card| card.id().and_then(|id| store.get(&id)))
                    .map(|state| state.total_reviews)
                    .sum();
                (store.len(), reviews)
            }
            Err(_) => (0, 0),
        };
    Some(DeckInventory {
        deck_hash,
        cards: deck.cards.len(),
        progress,
        reviews,
    })
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn zip_contents<W: Write + Seek>(root: &Path, output: W) -> Result<usize> {
    let mut archive = zip::ZipWriter::new(output);
    let mut paths = Vec::new();
    collect_archive_files(root, &mut paths)?;
    paths.sort_by_key(|path| {
        let name = archive_name(root, path);
        let group = match name.as_str() {
            "report.md" => 0,
            name if name.starts_with("log/") => 1,
            "config.toml" => 2,
            "decks.txt" => 3,
            _ => 4,
        };
        (group, name)
    });
    for path in &paths {
        archive.start_file(
            archive_name(root, path),
            zip::write::SimpleFileOptions::default(),
        )?;
        archive.write_all(&fs::read(path)?)?;
    }
    archive.finish()?;
    Ok(paths.len())
}

fn collect_archive_files(dir: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_archive_files(&path, paths)?;
        } else if kind.is_file() {
            paths.push(path);
        }
    }
    Ok(())
}

fn archive_name(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::{io::Read, path::Path};

    use proptest::prelude::*;

    use super::*;

    fn archive_text(path: &Path) -> (Vec<String>, String) {
        let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut names = Vec::new();
        let mut all = String::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).unwrap();
            names.push(entry.name().to_string());
            let mut text = String::new();
            entry.read_to_string(&mut text).unwrap();
            all.push_str(&text);
        }
        (names, all)
    }

    proptest! {
        #[test]
        fn redaction_never_leaves_the_home_username_or_a_token(
            prefix in "[a-z]{0,20}",
            suffix in "[a-z]{0,20}",
            token in "[A-Za-z0-9]{8,40}",
        ) {
            let home = Path::new("/home/alex");
            let input = format!("{prefix} /home/alex/decks alex {token} {suffix}");

            let output = redact(&input, home, std::slice::from_ref(&token));

            prop_assert!(!output.contains("/home/alex"), "{output}");
            prop_assert!(!output.contains("alex"), "{output}");
            prop_assert!(!output.contains(&token), "{output}");
            prop_assert!(output.contains("~/decks"), "{output}");
        }
    }

    #[test]
    fn a_bundle_is_reviewable_and_contains_no_credentials_or_deck_content() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("decks");
        let out = dir.path().join("out");
        std::fs::create_dir_all(&root).unwrap();
        let deck = root.join("divorce-lawyer-questions.md");
        std::fs::write(
            &deck,
            "---\nformat-version: 1\nid: \"deck-0j2k4m6p8r1t3v5x7z9b2d4f6h\"\n---\n## private-front\nprivate-back\n> private-note\n<!-- id: card-1j3k5m7p9r2t4v6x8z0b3d5f7h -->\n",
        )
        .unwrap();
        std::fs::write(
            root.join("divorce-lawyer-questions.personal.md"),
            "private-personal-sidecar",
        )
        .unwrap();
        let config = dir.path().join("profile.toml");
        std::fs::write(
            &config,
            "decks_dir = \"/home/alex/decks\"\n[serve]\ntoken = \"live-token-123456\"\nport = 7777\n[generate]\nprompt = \"private-ai-prompt\"\nextra = \"private-ai-extra\"\n",
        )
        .unwrap();
        let log = dir.path().join("alix-profile.log");
        std::fs::write(
            &log,
            "target=error kind=http method=GET area=api status=500 future=private-log-field path=/home/alex/decks token=live-token-123456 user=alex\n",
        )
        .unwrap();
        std::fs::write(
            log.with_extension("log.1"),
            "target=select card=card-safe\n",
        )
        .unwrap();
        let log_paths = vec![log.clone()];

        let bundle = write_bundle_with(&BundleOptions {
            root: &root,
            out_dir: &out,
            config_path: Some(&config),
            log_paths: &log_paths,
            include_deck: None,
            home: Path::new("/home/alex"),
            tokens: &["live-token-123456".to_string()],
            now_ms: 1_786_377_600_000,
        })
        .unwrap();

        assert_eq!(
            "alix-bug-report-0.8.0-20260810T160000Z.zip",
            bundle.path.file_name().unwrap()
        );
        assert!(bundle.bytes > 0);
        let (names, text) = archive_text(&bundle.path);
        assert_eq!(
            vec![
                "report.md",
                "log/alix.log",
                "log/alix.log.1",
                "config.toml",
                "decks.txt",
            ],
            names
        );
        assert_eq!(names.len(), bundle.files);
        for private in [
            "live-token-123456",
            "/home/alex",
            "alex",
            "divorce-lawyer-questions",
            "private-front",
            "private-back",
            "private-note",
            "private-personal-sidecar",
            "private-log-field",
            "private-ai-prompt",
            "private-ai-extra",
        ] {
            assert!(!text.contains(private), "bundle leaked {private:?}: {text}");
        }
        assert!(text.contains("decks_dir = \"~/decks\""), "{text}");
        assert!(!text.contains("token ="), "{text}");
        assert!(text.contains("cards=1"), "{text}");
    }

    #[test]
    fn identical_inputs_produce_byte_identical_archives() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("decks");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("b.md"), "## b\n2\n").unwrap();
        std::fs::write(root.join("a.md"), "## a\n1\n").unwrap();
        let first_out = dir.path().join("one");
        let second_out = dir.path().join("two");
        let options = |out_dir| BundleOptions {
            root: &root,
            out_dir,
            config_path: None,
            log_paths: &[],
            include_deck: None,
            home: Path::new("/home/alex"),
            tokens: &[],
            now_ms: 1_786_377_600_000,
        };

        let first = write_bundle_with(&options(&first_out)).unwrap();
        let second = write_bundle_with(&options(&second_out)).unwrap();

        assert_eq!(
            std::fs::read(first.path).unwrap(),
            std::fs::read(second.path).unwrap()
        );
    }

    #[test]
    fn a_deck_hash_survives_a_private_filename_change() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("divorce-lawyer-questions.md");
        let second = dir.path().join("new-private-name.md");
        std::fs::write(
            &first,
            "---\nformat-version: 1\nid: \"deck-0j2k4m6p8r1t3v5x7z9b2d4f6h\"\n---\n## front\nback\n<!-- id: card-1j3k5m7p9r2t4v6x8z0b3d5f7h -->\n",
        )
        .unwrap();

        let before = deck_inventory(dir.path()).remove(0).deck_hash;
        std::fs::rename(&first, &second).unwrap();
        let after = deck_inventory(dir.path()).remove(0).deck_hash;

        assert_eq!(before, after);
        assert_eq!(hex_digest(b"deck-0j2k4m6p8r1t3v5x7z9b2d4f6h"), after);
        assert_ne!(hex_digest(b"divorce-lawyer-questions.md"), after);
    }
}
