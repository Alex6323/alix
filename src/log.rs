#[cfg(test)]
use std::cell::RefCell;
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Mutex, Once, OnceLock},
};

use sha2::{Digest, Sha256};

pub const DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024;

static SINK: OnceLock<Sink> = OnceLock::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "full", derive(clap::ValueEnum))]
pub enum Target {
    Error,
    Http,
    Select,
}

impl FromStr for Target {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "error" => Ok(Self::Error),
            "http" => Ok(Self::Http),
            "select" => Ok(Self::Select),
            _ => Err(format!(
                "unknown log target {value:?}: expected error, http, or select"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Targets {
    error: bool,
    http: bool,
    select: bool,
}

impl Targets {
    pub fn from_slice(targets: &[Target]) -> Self {
        Self {
            error: targets.contains(&Target::Error),
            http: targets.contains(&Target::Http),
            select: targets.contains(&Target::Select),
        }
    }

    fn contains(self, target: Target) -> bool {
        match target {
            Target::Error => self.error,
            Target::Http => self.http,
            Target::Select => self.select,
        }
    }
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Http => "http",
            Self::Select => "select",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub max_bytes: u64,
    pub verbose: bool,
    pub stderr: Targets,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_BYTES,
            verbose: false,
            stderr: Targets::default(),
        }
    }
}

struct Sink {
    writer: Mutex<CappedWriter>,
    verbose: bool,
    stderr: Targets,
}

impl Sink {
    fn file_enabled(&self, target: Target) -> bool {
        self.verbose || matches!(target, Target::Error | Target::Select)
    }

    fn enabled(&self, target: Target) -> bool {
        self.file_enabled(target) || self.stderr.contains(target)
    }
}

pub fn enabled(target: Target) -> bool {
    SINK.get().is_some_and(|sink| sink.enabled(target))
}

pub fn emit(target: Target, fields: fmt::Arguments<'_>) {
    let sink = SINK.get().filter(|sink| sink.enabled(target));
    #[cfg(test)]
    let capturing = capture_enabled();
    #[cfg(not(test))]
    let capturing = false;
    if sink.is_none() && !capturing {
        return;
    }
    let line = format_line(target, fields);
    #[cfg(test)]
    capture_line(&line);
    if let Some(sink) = sink {
        write_to_sink(sink, target, &line);
    }
}

#[cfg(test)]
fn emit_to(sink: &Sink, target: Target, fields: fmt::Arguments<'_>) {
    if !sink.enabled(target) {
        return;
    }
    let line = format_line(target, fields);
    write_to_sink(sink, target, &line);
}

fn write_to_sink(sink: &Sink, target: Target, line: &str) {
    if sink.file_enabled(target)
        && let Ok(mut writer) = sink.writer.lock()
    {
        let _ = writer.write_all(line.as_bytes());
    }
    if sink.stderr.contains(target) {
        let _ = std::io::stderr().write_all(line.as_bytes());
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Ai,
    Http,
    Panic,
    Parse,
}

impl ErrorKind {
    fn name(self) -> &'static str {
        match self {
            Self::Ai => "ai",
            Self::Http => "http",
            Self::Panic => "panic",
            Self::Parse => "parse",
        }
    }
}

pub fn error(kind: ErrorKind, fields: fmt::Arguments<'_>) {
    emit(Target::Error, format_args!("kind={} {fields}", kind.name()));
}

fn record_panic(file: &str, line: u32, thread: &str) {
    error(
        ErrorKind::Panic,
        format_args!("file={file} line={line} thread={thread}"),
    );
}

fn install_panic_hook() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let (file, line) = info.location().map_or(("unknown", 0), |location| {
                (location.file(), location.line())
            });
            let thread = std::thread::current();
            record_panic(file, line, thread.name().unwrap_or("unnamed"));
            previous(info);
        }));
    });
}

#[cfg(test)]
thread_local! {
    static CAPTURE_LINES: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn capture(work: impl FnOnce()) -> Vec<String> {
    struct ResetCapture;
    impl Drop for ResetCapture {
        fn drop(&mut self) {
            CAPTURE_LINES.with(|lines| {
                lines.borrow_mut().take();
            });
        }
    }

    CAPTURE_LINES.with(|lines| {
        assert!(
            lines.borrow_mut().replace(Vec::new()).is_none(),
            "diagnostic captures cannot be nested"
        );
    });
    let reset = ResetCapture;
    work();
    let captured = CAPTURE_LINES.with(|lines| lines.borrow_mut().take().unwrap_or_default());
    drop(reset);
    captured
}

#[cfg(test)]
fn capture_enabled() -> bool {
    CAPTURE_LINES.with(|lines| lines.borrow().is_some())
}

#[cfg(test)]
fn capture_line(line: &str) {
    CAPTURE_LINES.with(|lines| {
        if let Some(lines) = lines.borrow_mut().as_mut() {
            lines.push(line.to_string());
        }
    });
}

fn format_line(target: Target, fields: fmt::Arguments<'_>) -> String {
    format!("target={} {fields}\n", target.name())
}

pub fn log_path(instance: &str) -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "alix")
        .map(|dirs| log_path_in(dirs.state_dir(), dirs.data_dir(), instance))
}

pub fn log_paths() -> io::Result<Vec<PathBuf>> {
    let Some(dirs) = directories::ProjectDirs::from("", "", "alix") else {
        return Ok(Vec::new());
    };
    log_paths_in(dirs.state_dir().unwrap_or(dirs.data_dir()))
}

fn log_paths_in(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("alix-") && name.ends_with(".log") && entry.file_type()?.is_file() {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

fn log_path_in(state_dir: Option<&Path>, data_dir: &Path, instance: &str) -> PathBuf {
    state_dir
        .unwrap_or(data_dir)
        .join(instance_file_name(instance))
}

fn instance_file_name(instance: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(instance.as_bytes());
    let label = readable_instance_label(instance);
    let mut name = String::with_capacity(18 + label.len());
    name.push_str("alix-");
    name.push_str(&label);
    name.push('-');
    for byte in &digest[..4] {
        name.push(HEX[(byte >> 4) as usize] as char);
        name.push(HEX[(byte & 0x0f) as usize] as char);
    }
    name.push_str(".log");
    name
}

fn readable_instance_label(instance: &str) -> String {
    if let Some((kind, _)) = instance.split_once(':')
        && matches!(kind, "config" | "scoped")
    {
        return kind.into();
    }
    if instance.contains(['/', '\\']) {
        return "profile".into();
    }

    let mut label = String::new();
    let mut pending_separator = false;
    let mut characters = 0;
    for character in instance.chars() {
        if character.is_alphanumeric() {
            if pending_separator && !label.is_empty() && characters < 32 {
                label.push('-');
                characters += 1;
            }
            pending_separator = false;
            for lowercase in character.to_lowercase() {
                if characters == 32 {
                    break;
                }
                label.push(lowercase);
                characters += 1;
            }
        } else if !label.is_empty() {
            pending_separator = true;
        }
    }
    if label.is_empty() {
        "profile".into()
    } else {
        label
    }
}

pub fn init(instance: &str, settings: Settings) -> io::Result<()> {
    if SINK.get().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the server log is already initialized",
        ));
    }
    let path = log_path(instance).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "the platform state directory is unavailable",
        )
    })?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the log path has no parent"))?;
    fs::create_dir_all(parent)?;
    let writer = CappedWriter::open(path, settings.max_bytes)?;
    SINK.set(Sink {
        writer: Mutex::new(writer),
        verbose: settings.verbose,
        stderr: settings.stderr,
    })
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the server log is already initialized",
        )
    })?;
    install_panic_hook();
    Ok(())
}

struct CappedWriter {
    path: PathBuf,
    file: Option<File>,
    bytes: u64,
    max_bytes: u64,
}

impl CappedWriter {
    fn open(path: PathBuf, max_bytes: u64) -> io::Result<Self> {
        if max_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the log size cap must be positive",
            ));
        }
        let file = open_file(&path)?;
        let bytes = file.metadata()?.len();
        Ok(Self {
            path,
            file: Some(file),
            bytes,
            max_bytes,
        })
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.take();
        let rolled = self.path.with_extension("log.1");
        ignore_not_found(fs::remove_file(&rolled))?;
        fs::rename(&self.path, rolled)?;
        self.file = Some(open_file(&self.path)?);
        self.bytes = 0;
        Ok(())
    }
}

fn ignore_not_found(result: io::Result<()>) -> io::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

impl Write for CappedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.bytes.saturating_add(buf.len() as u64) > self.max_bytes {
            self.rotate()?;
        }
        // If the buffer fit beside the current bytes, it is smaller than the
        // cap. If it did not, rotate() reset bytes to zero. Either way the cap,
        // not `cap - bytes`, is the only slice bound still needed here.
        let write_len = usize::try_from(self.max_bytes)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let Some(file) = self.file.as_mut() else {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the log file is unavailable",
            ));
        };
        let written = file.write(&buf[..write_len])?;
        self.bytes += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the log file is unavailable",
            )),
        }
    }
}

fn open_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

#[cfg(test)]
mod tests {
    use std::{fmt, io::Write, path::Path};

    use super::*;

    struct PanicIfFormatted;

    impl fmt::Display for PanicIfFormatted {
        fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
            panic!("disabled emission formatted its fields")
        }
    }

    #[test]
    fn emission_without_an_installed_sink_does_not_format() {
        emit(Target::Select, format_args!("card={PanicIfFormatted}"));
    }

    #[test]
    fn each_target_formats_one_exact_key_value_line() {
        assert_eq!(
            "target=http at=12 took=3 w=1\n",
            format_line(Target::Http, format_args!("at=12 took=3 w=1"))
        );
        assert_eq!(
            "target=select card=card-a fresh=1\n",
            format_line(Target::Select, format_args!("card=card-a fresh=1"))
        );
    }

    #[test]
    fn target_names_parse_exactly_without_a_level_ladder() {
        assert_eq!(Ok(Target::Error), "error".parse());
        assert_eq!(Ok(Target::Http), "http".parse());
        assert_eq!(Ok(Target::Select), "select".parse());
        assert!("debug".parse::<Target>().is_err());
    }

    #[test]
    fn target_sets_enable_only_the_targets_the_user_named() {
        let http = Targets::from_slice(&[Target::Http]);
        assert!(!http.contains(Target::Error));
        assert!(http.contains(Target::Http));
        assert!(!http.contains(Target::Select));

        let select = Targets::from_slice(&[Target::Select]);
        assert!(!select.contains(Target::Error));
        assert!(!select.contains(Target::Http));
        assert!(select.contains(Target::Select));

        let error = Targets::from_slice(&[Target::Error]);
        assert!(error.contains(Target::Error));
        assert!(!error.contains(Target::Http));
        assert!(!error.contains(Target::Select));
    }

    #[test]
    fn logging_without_an_installed_sink_stays_disabled() {
        assert!(!enabled(Target::Http));
    }

    #[test]
    fn a_quiet_sink_enables_selection_but_not_http() {
        let dir = tempfile::tempdir().unwrap();
        let sink = Sink {
            writer: Mutex::new(CappedWriter::open(dir.path().join("alix.log"), 8).unwrap()),
            verbose: false,
            stderr: Targets::default(),
        };
        assert!(!sink.enabled(Target::Http));
        assert!(sink.enabled(Target::Select));
    }

    #[test]
    fn quiet_logging_records_diagnostics_but_not_request_timings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("alix.log");
        let sink = Sink {
            writer: Mutex::new(CappedWriter::open(path.clone(), 1024).unwrap()),
            verbose: false,
            stderr: Targets::default(),
        };

        emit_to(
            &sink,
            Target::Error,
            format_args!("kind=parse code=front_without_answer line=7"),
        );
        emit_to(&sink, Target::Http, format_args!("at=1ms took=2ms w=0"));

        assert_eq!(
            "target=error kind=parse code=front_without_answer line=7\n",
            std::fs::read_to_string(path).unwrap()
        );
    }

    #[test]
    fn panic_diagnostics_omit_the_panic_payload() {
        let lines = capture(|| {
            record_panic("src/serve/study.rs", 42, "study-owner");
        });

        assert_eq!(
            vec![
                "target=error kind=panic file=src/serve/study.rs line=42 thread=study-owner\n"
                    .to_string()
            ],
            lines
        );
        assert!(lines.iter().all(|line| !line.contains("payload")));
    }

    #[test]
    fn log_path_prefers_state_and_falls_back_to_data() {
        let state = log_path_in(Some(Path::new("/state")), Path::new("/data"), "profile-a");
        let data = log_path_in(None, Path::new("/data"), "profile-a");

        assert_eq!(Some(Path::new("/state")), state.parent());
        assert_eq!(Some(Path::new("/data")), data.parent());
        assert_eq!(state.file_name(), data.file_name());
    }

    #[test]
    fn log_listing_finds_each_instance_but_not_its_rollover() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alix-anna-a.log"), "anna").unwrap();
        std::fs::write(dir.path().join("alix-anna-a.log.1"), "old anna").unwrap();
        std::fs::write(dir.path().join("alix-timmy-b.log"), "timmy").unwrap();
        std::fs::write(dir.path().join("other.log"), "other").unwrap();

        assert_eq!(
            vec![
                dir.path().join("alix-anna-a.log"),
                dir.path().join("alix-timmy-b.log")
            ],
            log_paths_in(dir.path()).unwrap()
        );
    }

    #[test]
    fn instance_names_are_stable_distinct_and_cannot_escape_the_state_directory() {
        let state = Path::new("/state");
        let anna = log_path_in(Some(state), Path::new("/data"), "anna");
        let anna_again = log_path_in(Some(state), Path::new("/data"), "anna");
        let timmy = log_path_in(Some(state), Path::new("/data"), "timmy");
        let escape = log_path_in(Some(state), Path::new("/data"), "../evil");

        assert_eq!(anna, anna_again);
        assert_ne!(anna, timmy);
        assert_eq!(Some(state), escape.parent());
        let name = escape.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("alix-") && name.ends_with(".log"));
        assert!(!name.contains("evil") && !name.contains(".."));
    }

    #[test]
    fn readable_instance_name_survives_with_a_digest_to_disambiguate_collisions() {
        let amelie = instance_file_name("Amelie");
        let config = instance_file_name("config:/private/Amelie/decks.toml");

        assert!(amelie.starts_with("alix-amelie-"));
        assert!(amelie.ends_with(".log"));
        assert_ne!(instance_file_name("a b"), instance_file_name("a-b"));
        assert!(config.starts_with("alix-config-") && config.ends_with(".log"));
        assert!(!config.contains("private") && !config.contains("Amelie"));
    }

    #[test]
    fn readable_instance_labels_normalize_separators_and_stop_at_32_characters() {
        assert_eq!("anna-marie", readable_instance_label("..Anna--Marie.."));

        let thirty_two = "a".repeat(32);
        assert_eq!(thirty_two, readable_instance_label(&"a".repeat(40)));

        let thirty_one_then_separator = format!("{}-", "a".repeat(31));
        assert_eq!(
            thirty_one_then_separator,
            readable_instance_label(&format!("{} b", "a".repeat(31)))
        );
    }

    #[test]
    fn filling_the_cap_exactly_does_not_rotate_an_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("alix.log");
        let mut writer = CappedWriter::open(path.clone(), 8).unwrap();

        writer.write_all(b"12345678").unwrap();

        assert_eq!(b"12345678", std::fs::read(&path).unwrap().as_slice());
        assert!(!path.with_extension("log.1").exists());
    }

    #[test]
    fn a_writer_left_without_a_file_reports_flush_failure() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = CappedWriter::open(dir.path().join("alix.log"), 8).unwrap();
        writer.file.take();

        assert_eq!(
            io::ErrorKind::BrokenPipe,
            writer.flush().unwrap_err().kind()
        );
    }

    #[test]
    fn only_a_missing_rolled_file_is_safe_to_ignore() {
        assert!(ignore_not_found(Ok(())).is_ok());
        assert!(ignore_not_found(Err(io::ErrorKind::NotFound.into())).is_ok());
        assert_eq!(
            io::ErrorKind::PermissionDenied,
            ignore_not_found(Err(io::ErrorKind::PermissionDenied.into()))
                .unwrap_err()
                .kind()
        );
    }

    #[test]
    fn capped_writer_keeps_only_two_bounded_files_and_newest_content_current() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("alix.log");
        let mut writer = CappedWriter::open(path.clone(), 8).unwrap();

        writer.write_all(b"one\ntwo\n").unwrap();
        writer.write_all(b"three\n").unwrap();
        writer.write_all(b"four\n").unwrap();

        let mut names = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            vec![
                std::ffi::OsString::from("alix.log"),
                std::ffi::OsString::from("alix.log.1")
            ],
            names
        );
        assert_eq!(b"four\n", std::fs::read(&path).unwrap().as_slice());
        assert_eq!(
            b"three\n",
            std::fs::read(path.with_extension("log.1"))
                .unwrap()
                .as_slice()
        );
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            assert!(entry.unwrap().metadata().unwrap().len() <= 8);
        }
    }

    #[test]
    fn different_profile_writers_never_share_a_file_or_byte_cap() {
        let dir = tempfile::tempdir().unwrap();
        let anna = log_path_in(Some(dir.path()), dir.path(), "anna");
        let timmy = log_path_in(Some(dir.path()), dir.path(), "timmy");
        assert_ne!(anna, timmy);
        let mut first = CappedWriter::open(anna.clone(), 8).unwrap();
        let mut second = CappedWriter::open(timmy.clone(), 8).unwrap();

        first.write_all(b"one\ntwo\n").unwrap();
        second.write_all(b"three\n").unwrap();

        assert!(std::fs::metadata(anna).unwrap().len() <= 8);
        assert!(std::fs::metadata(timmy).unwrap().len() <= 8);
    }
}
