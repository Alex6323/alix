use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use sha2::{Digest, Sha256};

pub const DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024;

static SINK: OnceLock<Sink> = OnceLock::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target {
    Http,
    Select,
}

impl Target {
    fn name(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Select => "select",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub max_bytes: u64,
    pub verbose: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_BYTES,
            verbose: false,
        }
    }
}

struct Sink {
    writer: Mutex<CappedWriter>,
    verbose: bool,
}

impl Sink {
    fn enabled(&self, target: Target) -> bool {
        self.verbose || target == Target::Select
    }
}

pub fn enabled(target: Target) -> bool {
    SINK.get().is_some_and(|sink| sink.enabled(target))
}

pub fn emit(target: Target, fields: fmt::Arguments<'_>) {
    let Some(sink) = SINK.get().filter(|sink| sink.enabled(target)) else {
        return;
    };
    let Ok(mut writer) = sink.writer.lock() else {
        return;
    };
    let line = format_line(target, fields);
    let _ = writer.write_all(line.as_bytes());
}

fn format_line(target: Target, fields: fmt::Arguments<'_>) -> String {
    format!("target={} {fields}\n", target.name())
}

pub fn log_path(instance: &str) -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "alix")
        .map(|dirs| log_path_in(dirs.state_dir(), dirs.data_dir(), instance))
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
    })
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the server log is already initialized",
        )
    })
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
        match fs::remove_file(&rolled) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::rename(&self.path, rolled)?;
        self.file = Some(open_file(&self.path)?);
        self.bytes = 0;
        Ok(())
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
        let remaining = (self.max_bytes - self.bytes) as usize;
        let Some(file) = self.file.as_mut() else {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the log file is unavailable",
            ));
        };
        let written = file.write(&buf[..buf.len().min(remaining)])?;
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
    fn log_path_prefers_state_and_falls_back_to_data() {
        let state = log_path_in(Some(Path::new("/state")), Path::new("/data"), "profile-a");
        let data = log_path_in(None, Path::new("/data"), "profile-a");

        assert_eq!(Some(Path::new("/state")), state.parent());
        assert_eq!(Some(Path::new("/data")), data.parent());
        assert_eq!(state.file_name(), data.file_name());
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

        assert!(amelie.starts_with("alix-amelie-"));
        assert!(amelie.ends_with(".log"));
        assert_ne!(instance_file_name("a b"), instance_file_name("a-b"));
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
