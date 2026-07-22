use std::{
    fs,
    path::{Path, PathBuf},
};

use alix::config::{Audience, Config};
use anyhow::{Context, Result, bail};

use crate::{ProfileAddArgs, ProfileCommand, ProfileDefaultArgs, common::confirm};

pub(crate) const RESERVED: [&str; 4] = ["add", "list", "remove", "default"];

pub(crate) fn profiles_dir() -> Result<PathBuf> {
    alix::config::profiles_dir().context("cannot determine the alix config directory")
}

pub(crate) fn config_path_in(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.toml"))
}

pub(crate) fn default_marker_in(dir: &Path) -> PathBuf {
    dir.join("default")
}

pub(crate) fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains(['/', '\\', '.']) {
        bail!("a profile name cannot be empty or contain `/`, `\\`, or `.`");
    }
    if RESERVED.contains(&name) {
        bail!(
            "`{name}` is a reserved profile command; pick another name (reserved: {})",
            RESERVED.join(", ")
        );
    }
    Ok(())
}

pub(crate) fn run(cmd: ProfileCommand) -> Result<()> {
    match cmd {
        ProfileCommand::Add(args) => add(args),
        ProfileCommand::List => list(),
        ProfileCommand::Remove(args) => remove(&args.name, args.yes),
        ProfileCommand::Default(args) => default_cmd(args),
        ProfileCommand::Launch(args) => launch_named(args),
    }
}

fn add(args: ProfileAddArgs) -> Result<()> {
    let dir = profiles_dir()?;
    let config = Config::load(None)?;
    add_in(&dir, &args, &config)
}

fn add_in(dir: &Path, args: &ProfileAddArgs, config: &Config) -> Result<()> {
    validate_name(&args.name)?;
    let path = config_path_in(dir, &args.name);
    if path.exists() {
        bail!("profile `{}` already exists; remove it first", args.name);
    }

    let decks = args
        .decks
        .clone()
        .or_else(|| config.decks_dir())
        .context("cannot determine the decks directory")?;
    let decks = decks
        .to_str()
        .context("the decks path is not valid UTF-8")?;
    let port = args.port.unwrap_or(config.serve.port);
    let audience = match (args.kids, args.adult) {
        (true, _) => "kids",
        _ => "adult",
    };
    let token = crate::launch::generate_token()?;
    let toml = format!(
        "decks_dir = {decks:?}\n\n[serve]\nport = {port}\naudience = \"{audience}\"\ntoken = \"{token}\"\n"
    );

    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    write_atomic(&path, &toml)?;
    println!("wrote profile `{}` to {}", args.name, path.display());
    if args.kids {
        println!("this profile launches on your LAN with a stable token; bookmark its printed URL");
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct ProfileRow {
    name: String,
    audience: Audience,
    port: u16,
    decks: PathBuf,
}

fn list() -> Result<()> {
    let dir = profiles_dir()?;
    let rows = profile_rows_in(&dir)?;
    if rows.is_empty() {
        println!("no profiles yet; create one with `alix profile add <name>`");
        return Ok(());
    }

    let name_width = rows
        .iter()
        .map(|row| row.name.len())
        .max()
        .unwrap_or_default()
        .max("name".len());
    println!(
        "{:<name_width$}  {:<6}  {:>5}  decks",
        "name", "flavor", "port"
    );
    for row in rows {
        let audience = match row.audience {
            Audience::Adult => "adult",
            Audience::Kids => "kids",
        };
        println!(
            "{:<name_width$}  {audience:<6}  {:>5}  {}",
            row.name,
            row.port,
            row.decks.display()
        );
    }
    Ok(())
}

fn profile_rows_in(dir: &Path) -> Result<Vec<ProfileRow>> {
    let mut rows = Vec::new();
    for path in profile_paths_in(dir)? {
        let name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .context("a profile filename is not valid UTF-8")?
            .to_string();
        let config = Config::load(Some(&path))?;
        let decks = config
            .decks_dir()
            .context("cannot determine the decks directory")?;
        rows.push(ProfileRow {
            name,
            audience: config.serve.audience,
            port: config.serve.port,
            decks,
        });
    }
    Ok(rows)
}

fn profile_paths_in(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let path = entry?.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "toml")
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn remove(name: &str, yes: bool) -> Result<()> {
    let dir = profiles_dir()?;
    remove_in(&dir, name, yes)
}

fn remove_in(dir: &Path, name: &str, yes: bool) -> Result<()> {
    validate_name(name)?;
    let path = config_path_in(dir, name);
    if !path.exists() {
        bail!("no profile `{name}` (looked in {})", path.display());
    }
    if !confirm(&format!("Delete profile `{name}`?"), yes)? {
        return Ok(());
    }
    fs::remove_file(&path).with_context(|| format!("cannot remove {}", path.display()))?;
    println!("removed profile `{name}`");
    Ok(())
}

fn default_cmd(args: ProfileDefaultArgs) -> Result<()> {
    let dir = profiles_dir()?;
    default_cmd_in(&dir, args)
}

fn default_cmd_in(dir: &Path, args: ProfileDefaultArgs) -> Result<()> {
    let marker = default_marker_in(dir);
    if args.clear {
        if marker.exists() {
            fs::remove_file(&marker)
                .with_context(|| format!("cannot remove {}", marker.display()))?;
        }
        println!("default profile cleared");
        return Ok(());
    }

    if let Some(name) = args.name {
        validate_name(&name)?;
        let path = config_path_in(dir, &name);
        if !path.exists() {
            bail!("no profile `{name}` (looked in {})", path.display());
        }
        fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        write_atomic(&marker, &format!("{name}\n"))?;
        println!("default profile: {name}");
        return Ok(());
    }

    match read_default_in(dir)? {
        Some(name) => println!("{name}"),
        None => println!("none"),
    }
    Ok(())
}

fn read_default_in(dir: &Path) -> Result<Option<String>> {
    let marker = default_marker_in(dir);
    if !marker.exists() {
        return Ok(None);
    }
    let name =
        fs::read_to_string(&marker).with_context(|| format!("cannot read {}", marker.display()))?;
    let name = name.trim();
    if name.is_empty() {
        Ok(None)
    } else {
        Ok(Some(name.to_string()))
    }
}

fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents).with_context(|| format!("cannot write {}", path.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("cannot write {}", path.display()))
}

fn launch_named(args: Vec<String>) -> Result<()> {
    bail!("profile launch is not implemented yet: {}", args.join(" "))
}

#[cfg(test)]
mod tests {
    use alix::config::ServeConfig;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn reserved_verbs_are_rejected_as_names() {
        for verb in RESERVED {
            assert!(validate_name(verb).is_err(), "{verb} should be reserved");
        }
        assert!(validate_name("timmy").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("a/b").is_err());
    }

    #[test]
    fn config_path_is_name_dot_toml_under_the_dir() {
        let dir = Path::new("/cfg/profiles");
        assert_eq!(dir.join("timmy.toml"), config_path_in(dir, "timmy"));
        assert_eq!(dir.join("default"), default_marker_in(dir));
    }

    #[test]
    fn profile_management_round_trips_configs_rows_and_removal() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("profiles");
        let decks = temp.path().join("timmy-decks");
        let config = Config {
            decks_dir: Some(temp.path().join("fallback-decks")),
            serve: ServeConfig {
                port: 9000,
                ..ServeConfig::default()
            },
            ..Config::default()
        };
        let args = ProfileAddArgs {
            name: "timmy".to_string(),
            decks: Some(decks.clone()),
            port: Some(7002),
            kids: true,
            adult: false,
        };

        add_in(&dir, &args, &config).unwrap();
        let path = config_path_in(&dir, "timmy");
        let loaded = Config::load(Some(&path)).unwrap();
        assert_eq!(Some(decks.clone()), loaded.decks_dir);
        assert_eq!(7002, loaded.serve.port);
        assert_eq!(Audience::Kids, loaded.serve.audience);
        assert!(loaded.serve.token.is_some_and(|token| !token.is_empty()));
        assert!(add_in(&dir, &args, &config).is_err());

        assert_eq!(
            vec![ProfileRow {
                name: "timmy".to_string(),
                audience: Audience::Kids,
                port: 7002,
                decks,
            }],
            profile_rows_in(&dir).unwrap()
        );

        remove_in(&dir, "timmy", true).unwrap();
        assert!(!path.exists());
        assert!(profile_rows_in(&dir).unwrap().is_empty());
    }

    #[test]
    fn add_rejects_a_reserved_profile_name() {
        let temp = TempDir::new().unwrap();
        let config = Config {
            decks_dir: Some(temp.path().join("decks")),
            ..Config::default()
        };
        let args = ProfileAddArgs {
            name: "add".to_string(),
            decks: None,
            port: None,
            kids: false,
            adult: false,
        };

        assert!(add_in(temp.path(), &args, &config).is_err());
    }

    #[test]
    fn default_set_show_and_clear_round_trip_the_marker() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("profiles");
        fs::create_dir_all(&dir).unwrap();
        fs::write(config_path_in(&dir, "timmy"), "").unwrap();

        default_cmd_in(
            &dir,
            ProfileDefaultArgs {
                name: Some("timmy".to_string()),
                clear: false,
            },
        )
        .unwrap();
        assert_eq!(Some("timmy".to_string()), read_default_in(&dir).unwrap());

        default_cmd_in(
            &dir,
            ProfileDefaultArgs {
                name: None,
                clear: false,
            },
        )
        .unwrap();
        default_cmd_in(
            &dir,
            ProfileDefaultArgs {
                name: None,
                clear: true,
            },
        )
        .unwrap();
        assert_eq!(None, read_default_in(&dir).unwrap());
    }
}
