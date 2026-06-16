//! User configuration, loaded from a TOML file
//! (`~/.config/flash/config.toml` on Linux).
//!
//! Currently this configures key bindings. Every action takes a list of
//! keys; the first one is shown in the footer of the TUI. A key is written
//! as a single character (`"j"`), a special key name (`"space"`, `"enter"`,
//! `"tab"`, `"esc"`, `"backspace"`), or either with a `ctrl-` prefix
//! (`"ctrl-s"`).
//!
//! Plain-character bindings are ignored while you are typing an answer
//! (typing and fuzzy mode), so they cannot shadow text input; use `ctrl-` or
//! special keys for actions that must be reachable there (hint, skip, quit).

use std::{
    fmt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// A key without modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    Esc,
    Backspace,
}

/// A key plus the Ctrl modifier flag; what a binding matches against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPattern {
    pub key: Key,
    pub ctrl: bool,
}

impl KeyPattern {
    /// `true` if this pattern would swallow plain text input.
    pub fn is_plain_char(&self) -> bool {
        matches!(self.key, Key::Char(_)) && !self.ctrl
    }
}

impl fmt::Display for KeyPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            write!(f, "Ctrl-")?;
        }
        match self.key {
            Key::Char(' ') => write!(f, "SPACE"),
            // After "Ctrl-" the letter reads better uppercased (Ctrl-S).
            Key::Char(c) if self.ctrl => write!(f, "{}", c.to_uppercase()),
            Key::Char(c) => write!(f, "{c}"),
            Key::Enter => write!(f, "ENTER"),
            Key::Tab => write!(f, "TAB"),
            Key::Esc => write!(f, "ESC"),
            Key::Backspace => write!(f, "BACKSPACE"),
        }
    }
}

/// Parses a key description like `"j"`, `"space"` or `"ctrl-s"`.
pub fn parse_key(s: &str) -> Result<KeyPattern> {
    let lower = s.trim().to_lowercase();
    let (ctrl, name) = match lower.strip_prefix("ctrl-") {
        Some(rest) => (true, rest),
        None => (false, lower.as_str()),
    };
    let key = match name {
        "space" => Key::Char(' '),
        "enter" | "return" => Key::Enter,
        "tab" => Key::Tab,
        "esc" | "escape" => Key::Esc,
        "backspace" => Key::Backspace,
        _ => {
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Key::Char(c),
                _ => bail!(
                    "invalid key {s:?}: expected a single character, \
                     space/enter/tab/esc/backspace, or a ctrl- prefix"
                ),
            }
        }
    };
    Ok(KeyPattern { key, ctrl })
}

/// All rebindable actions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bindings {
    /// Flip mode: grade as failed.
    pub again: Vec<KeyPattern>,
    /// Flip mode: grade as passed.
    pub good: Vec<KeyPattern>,
    /// Flip mode: grade as easy.
    pub easy: Vec<KeyPattern>,
    /// Flip mode: reveal the answer.
    pub reveal: Vec<KeyPattern>,
    /// Typing mode: reveal a hint (fails the card).
    pub hint: Vec<KeyPattern>,
    /// Fuzzy mode: submit the current line.
    pub submit: Vec<KeyPattern>,
    /// Put the current card at the end of the queue without grading.
    pub skip: Vec<KeyPattern>,
    /// Mark the current card for removal from its deck file (applied when the
    /// session ends).
    pub remove: Vec<KeyPattern>,
    /// Leave the feedback screen.
    pub cont: Vec<KeyPattern>,
    /// Start a new session from the summary screen.
    pub restart: Vec<KeyPattern>,
    /// Open the ask-Claude view on an answered card.
    pub ask: Vec<KeyPattern>,
    /// Ask view: condense the conversation and save it as a card note.
    pub save_note: Vec<KeyPattern>,
    /// Quit the session.
    pub quit: Vec<KeyPattern>,
}

impl Default for Bindings {
    fn default() -> Self {
        let keys = |list: &[&str]| list.iter().map(|s| parse_key(s).unwrap()).collect();
        Self {
            again: keys(&["1", "a"]),
            good: keys(&["2", "g"]),
            easy: keys(&["3", "e"]),
            reveal: keys(&["space", "enter"]),
            hint: keys(&["tab", "ctrl-h", "ctrl-backspace"]),
            submit: keys(&["enter"]),
            skip: keys(&["ctrl-s"]),
            remove: keys(&["ctrl-x"]),
            cont: keys(&["enter", "space"]),
            restart: keys(&["r"]),
            ask: keys(&["?"]),
            save_note: keys(&["ctrl-n"]),
            quit: keys(&["esc", "ctrl-c"]),
        }
    }
}

impl Bindings {
    /// The key shown in the footer for an action (its first binding).
    pub fn label(list: &[KeyPattern]) -> String {
        list.first()
            .map(|p| p.to_string())
            .unwrap_or_else(|| "?".to_string())
    }
}

/// Key bindings for the read-only browser (`flash browse`), configured in the
/// `[browse]` section. Jumping to the first/last card stays fixed at
/// `g`/`G`/Home/End — letter bindings are case-insensitive, so `g` and `G`
/// cannot be told apart — and the arrow keys always work for next/previous.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowseBindings {
    /// Move to the next card.
    pub next: Vec<KeyPattern>,
    /// Move to the previous card.
    pub prev: Vec<KeyPattern>,
    /// Mark the current card for removal from its deck file (applied on quit).
    pub remove: Vec<KeyPattern>,
    /// Leave the browser.
    pub quit: Vec<KeyPattern>,
}

impl Default for BrowseBindings {
    fn default() -> Self {
        let keys = |list: &[&str]| list.iter().map(|s| parse_key(s).unwrap()).collect();
        Self {
            next: keys(&["l", "n", "space"]),
            prev: keys(&["h", "p"]),
            remove: keys(&["x"]),
            quit: keys(&["q", "esc", "ctrl-c"]),
        }
    }
}

/// Settings for the ask-Claude integration (`[ask]` in the config file).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AskConfig {
    /// The CLI executable to run.
    pub command: String,
    /// Model passed as `--model`; `None` uses the CLI's own default.
    pub model: Option<String>,
    /// How long to wait for an answer before giving up.
    pub timeout_secs: u64,
    /// `--permission-mode` for the headless CLI. The default `"dontAsk"`
    /// silently denies any tool not in `allowed_tools` instead of waiting
    /// for an interactive approval that `-p` mode cannot provide (which
    /// would hang). An empty string omits the flag.
    pub permission_mode: String,
    /// Tools the assistant may use (`--allowedTools`). Combined with
    /// `dontAsk`, this is an exclusive allowlist: everything else is denied,
    /// so a malicious deck-link page cannot make the tutor run commands.
    pub allowed_tools: Vec<String>,
}

impl Default for AskConfig {
    fn default() -> Self {
        Self {
            command: "claude".to_string(),
            model: None,
            timeout_secs: 120,
            permission_mode: "dontAsk".to_string(),
            allowed_tools: vec!["WebFetch".to_string(), "WebSearch".to_string()],
        }
    }
}

/// Settings for AI deck generation (`flash generate`, the `[generate]` section).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerateConfig {
    /// Model passed as `--model`; `None` falls back to the `[ask]` model, then
    /// the CLI's own default.
    pub model: Option<String>,
    /// How long to wait for the deck before giving up (generation is a bigger
    /// call than a single question, so this is larger than the ask timeout).
    pub timeout_secs: u64,
    /// Upper bound on the number of cards to generate.
    pub max_cards: usize,
    /// Extra guidance appended to the instruction prompt (the common way to
    /// tweak generation, e.g. "focus on the public API").
    pub extra: Option<String>,
    /// A full replacement for the built-in instruction prompt. May use the
    /// `{url}` and `{max_cards}` placeholders.
    pub prompt: Option<String>,
    /// Run a second Claude pass that reviews the draft and removes redundant
    /// cards. `--review` forces it on for a single run.
    pub review: bool,
}

impl Default for GenerateConfig {
    fn default() -> Self {
        Self {
            model: None,
            timeout_secs: 300,
            max_cards: 30,
            extra: None,
            prompt: None,
            review: false,
        }
    }
}

/// Settings for the local web frontend (`flash serve`, the `[serve]` section).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServeConfig {
    /// Default port to listen on (overridden by `--port`).
    pub port: u16,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self { port: 7777 }
    }
}

/// The whole user configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub keys: Bindings,
    /// Key bindings for `flash browse`.
    pub browse: BrowseBindings,
    pub ask: AskConfig,
    /// AI deck generation settings.
    pub generate: GenerateConfig,
    /// Local web frontend settings.
    pub serve: ServeConfig,
    /// Directory the startup picker lists decks from, and resolves bare deck
    /// names against. `None` uses [`default_decks_dir`].
    pub decks_dir: Option<PathBuf>,
}

impl Config {
    /// The decks directory to use (config value or the default `~/decks`).
    pub fn decks_dir(&self) -> Option<PathBuf> {
        self.decks_dir.clone().or_else(default_decks_dir)
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    keys: RawKeys,
    #[serde(default)]
    browse: RawBrowse,
    #[serde(default)]
    ask: RawAsk,
    #[serde(default)]
    generate: RawGenerate,
    #[serde(default)]
    serve: RawServe,
    decks_dir: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawServe {
    port: Option<u16>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawGenerate {
    model: Option<String>,
    timeout_secs: Option<u64>,
    max_cards: Option<usize>,
    extra: Option<String>,
    prompt: Option<String>,
    review: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawBrowse {
    next: Option<Vec<String>>,
    prev: Option<Vec<String>>,
    remove: Option<Vec<String>>,
    quit: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawAsk {
    command: Option<String>,
    model: Option<String>,
    timeout_secs: Option<u64>,
    permission_mode: Option<String>,
    allowed_tools: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawKeys {
    again: Option<Vec<String>>,
    good: Option<Vec<String>>,
    easy: Option<Vec<String>>,
    reveal: Option<Vec<String>>,
    hint: Option<Vec<String>>,
    submit: Option<Vec<String>>,
    skip: Option<Vec<String>>,
    remove: Option<Vec<String>>,
    r#continue: Option<Vec<String>>,
    restart: Option<Vec<String>>,
    ask: Option<Vec<String>>,
    save_note: Option<Vec<String>>,
    quit: Option<Vec<String>>,
}

impl Config {
    /// Parses a configuration from TOML text. Actions that are not mentioned
    /// keep their default bindings; an empty list disables an action.
    pub fn from_toml(text: &str) -> Result<Self> {
        let raw: RawConfig = toml::from_str(text).context("invalid config file")?;
        let mut keys = Bindings::default();

        let assign = |target: &mut Vec<KeyPattern>,
                      source: Option<Vec<String>>,
                      action: &str|
         -> Result<()> {
            if let Some(list) = source {
                *target = list
                    .iter()
                    .map(|s| parse_key(s).with_context(|| format!("in binding for {action:?}")))
                    .collect::<Result<_>>()?;
            }
            Ok(())
        };

        assign(&mut keys.again, raw.keys.again, "again")?;
        assign(&mut keys.good, raw.keys.good, "good")?;
        assign(&mut keys.easy, raw.keys.easy, "easy")?;
        assign(&mut keys.reveal, raw.keys.reveal, "reveal")?;
        assign(&mut keys.hint, raw.keys.hint, "hint")?;
        assign(&mut keys.submit, raw.keys.submit, "submit")?;
        assign(&mut keys.skip, raw.keys.skip, "skip")?;
        assign(&mut keys.remove, raw.keys.remove, "remove")?;
        assign(&mut keys.cont, raw.keys.r#continue, "continue")?;
        assign(&mut keys.restart, raw.keys.restart, "restart")?;
        assign(&mut keys.ask, raw.keys.ask, "ask")?;
        assign(&mut keys.save_note, raw.keys.save_note, "save_note")?;
        assign(&mut keys.quit, raw.keys.quit, "quit")?;

        let mut browse = BrowseBindings::default();
        assign(&mut browse.next, raw.browse.next, "browse.next")?;
        assign(&mut browse.prev, raw.browse.prev, "browse.prev")?;
        assign(&mut browse.remove, raw.browse.remove, "browse.remove")?;
        assign(&mut browse.quit, raw.browse.quit, "browse.quit")?;

        let mut ask = AskConfig::default();
        if let Some(command) = raw.ask.command {
            ask.command = command;
        }
        // An empty model string means "use the CLI default", like absence.
        if let Some(model) = raw.ask.model.filter(|m| !m.trim().is_empty()) {
            ask.model = Some(model);
        }
        if let Some(secs) = raw.ask.timeout_secs {
            ask.timeout_secs = secs;
        }
        if let Some(mode) = raw.ask.permission_mode {
            ask.permission_mode = mode;
        }
        if let Some(tools) = raw.ask.allowed_tools {
            ask.allowed_tools = tools;
        }

        let mut generate = GenerateConfig::default();
        if let Some(model) = raw.generate.model.filter(|m| !m.trim().is_empty()) {
            generate.model = Some(model);
        }
        if let Some(secs) = raw.generate.timeout_secs {
            generate.timeout_secs = secs;
        }
        if let Some(max) = raw.generate.max_cards {
            generate.max_cards = max;
        }
        generate.extra = raw.generate.extra.filter(|s| !s.trim().is_empty());
        generate.prompt = raw.generate.prompt.filter(|s| !s.trim().is_empty());
        if let Some(review) = raw.generate.review {
            generate.review = review;
        }

        let mut serve = ServeConfig::default();
        if let Some(port) = raw.serve.port {
            serve.port = port;
        }

        let decks_dir = raw.decks_dir.map(|s| expand_tilde(&s));

        Ok(Self {
            keys,
            browse,
            ask,
            generate,
            serve,
            decks_dir,
        })
    }

    /// Loads the configuration.
    ///
    /// With an explicit `path` the file must exist. Without one, the default
    /// location is used if present, otherwise the default configuration is
    /// returned.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let path = match path {
            Some(path) => path.to_path_buf(),
            None => match default_config_path() {
                Some(path) if path.exists() => path,
                _ => return Ok(Self::default()),
            },
        };
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read config file {}", path.display()))?;
        Self::from_toml(&text).with_context(|| format!("in config file {}", path.display()))
    }
}

/// The default location of the config file
/// (`~/.config/flash/config.toml` on Linux).
pub fn default_config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "flash")
        .map(|dirs| dirs.config_dir().join("config.toml"))
}

/// The default decks directory (`~/decks`).
pub fn default_decks_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.home_dir().join("decks"))
}

/// Expands a leading `~` to the home directory.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(dirs) = directories::BaseDirs::new()
    {
        return dirs.home_dir().join(rest);
    }
    PathBuf::from(path)
}

/// A self-documenting template for `config --init`: every option is shown
/// commented out at its default value, so the emitted file overrides nothing
/// (uncomment a line to change it; defaults you leave commented still track
/// future versions). Section headers stay active so a single line can be
/// uncommented beneath one.
pub fn default_config_toml() -> &'static str {
    r#"# flash configuration.
#
# Every option below is shown commented out at its default value, as a
# reference. Uncomment a line and edit it to override that default; lines you
# leave commented keep the built-in default, so improvements to the defaults
# in newer versions still reach you. Keep the section headers ([keys],
# [browse], [ask], [generate], [serve]) so an uncommented line lands in the
# right section.
#
# Keys are written as a single character ("j"), a special key name
# ("space", "enter", "tab", "esc", "backspace"), or either with a "ctrl-"
# prefix ("ctrl-s"). The first key of each list is shown in the footer.
#
# Note: while you are typing an answer (typing and fuzzy mode), plain
# character bindings are ignored so they cannot shadow text input; use
# ctrl-/special keys for hint, skip and quit.

# Directory the startup picker lists decks from (when `flash` is launched
# without deck arguments). A leading ~ is expanded. Defaults to ~/decks.
# decks_dir = "~/decks"

# Review key bindings (flip / typing / fuzzy / choice modes).
[keys]
# again = ["1", "a"]            # flip mode: grade as failed
# good = ["2", "g"]             # flip mode: grade as passed
# easy = ["3", "e"]             # flip mode: grade as easy
# reveal = ["space", "enter"]   # flip mode: show the answer
# hint = ["tab", "ctrl-h", "ctrl-backspace"]  # typing mode (fails the card)
# submit = ["enter"]            # fuzzy mode: submit the current line
# skip = ["ctrl-s"]             # requeue the current card without grading
# remove = ["ctrl-x"]           # mark the card for removal from the deck file
# continue = ["enter", "space"] # leave the feedback screen
# restart = ["r"]               # start a new session from the summary screen
# ask = ["?"]                   # ask Claude about an answered card
# save_note = ["ctrl-n"]        # ask view: save a condensed note to the deck
# quit = ["esc", "ctrl-c"]      # quit the session

# Key bindings for `flash browse` (the read-only reader). Jumping to the first
# and last card is fixed to g / G / Home / End, and the arrow keys always
# move next/previous; these three are configurable:
[browse]
# next = ["l", "n", "space"]    # next card
# prev = ["h", "p"]             # previous card
# remove = ["x"]                # mark the card for removal from the deck file
# quit = ["q", "esc", "ctrl-c"] # leave the browser

# Settings for the ask-Claude integration. Questions are sent to the
# command below (the Claude Code CLI) together with the card as context.
[ask]
# command = "claude"            # executable to run
# model = ""                    # --model override; empty = the CLI's default
# timeout_secs = 120            # give up waiting after this many seconds
# Permission mode for the headless CLI. "dontAsk" silently denies any tool
# not listed below — no interactive prompt (which would hang -p mode).
# Other values: "bypassPermissions" (allow everything; unsafe), "default"
# (prompts, so it hangs headless). Empty omits the flag.
# permission_mode = "dontAsk"
# Tools the assistant may use. With "dontAsk" this is an exclusive
# allowlist; the defaults let it consult deck links but nothing else.
# allowed_tools = ["WebFetch", "WebSearch"]

# AI deck generation (`flash generate <url>`). Reuses the [ask] command,
# permission mode and tool allowlist (WebFetch reads the page).
[generate]
# model = ""                    # --model override; empty = use [ask] / CLI default
# timeout_secs = 300            # generation is slower than a single question
# max_cards = 30                # upper bound on cards per deck
# extra = ""                    # extra guidance appended to the prompt
# prompt = ""                   # full prompt override; may use {url} and {max_cards}
# review = false                # run a second pass to drop redundant cards (--review)

# Local web frontend (`flash serve`). Binds to localhost by default; `--lan`
# exposes it to the network and `--port` overrides the port set here.
[serve]
# port = 7777                   # default port for `flash serve`
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_chars() {
        assert_eq!(
            KeyPattern {
                key: Key::Char('j'),
                ctrl: false
            },
            parse_key("j").unwrap()
        );
        assert_eq!(
            KeyPattern {
                key: Key::Char('1'),
                ctrl: false
            },
            parse_key("1").unwrap()
        );
    }

    #[test]
    fn parse_special_keys() {
        assert_eq!(Key::Char(' '), parse_key("space").unwrap().key);
        assert_eq!(Key::Enter, parse_key("enter").unwrap().key);
        assert_eq!(Key::Enter, parse_key("return").unwrap().key);
        assert_eq!(Key::Tab, parse_key("tab").unwrap().key);
        assert_eq!(Key::Esc, parse_key("ESC").unwrap().key);
        assert_eq!(Key::Backspace, parse_key("backspace").unwrap().key);
    }

    #[test]
    fn parse_ctrl_combinations() {
        assert_eq!(
            KeyPattern {
                key: Key::Char('s'),
                ctrl: true
            },
            parse_key("ctrl-s").unwrap()
        );
        assert_eq!(
            KeyPattern {
                key: Key::Backspace,
                ctrl: true
            },
            parse_key("ctrl-backspace").unwrap()
        );
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_key("jj").is_err());
        assert!(parse_key("").is_err());
        assert!(parse_key("ctrl-").is_err());
        assert!(parse_key("super-x").is_err());
    }

    #[test]
    fn display_labels() {
        assert_eq!("j", parse_key("j").unwrap().to_string());
        assert_eq!("SPACE", parse_key("space").unwrap().to_string());
        assert_eq!("Ctrl-S", parse_key("ctrl-s").unwrap().to_string().as_str());
        assert_eq!("ESC", parse_key("esc").unwrap().to_string());
    }

    #[test]
    fn default_config_when_no_file() {
        let config = Config::load(None);
        // No assertion on the file system; defaults must always parse.
        assert!(config.is_ok());
        assert_eq!(Bindings::default(), Config::default().keys);
    }

    #[test]
    fn rebind_grades_to_jkl() {
        let config =
            Config::from_toml("[keys]\nagain = [\"j\"]\ngood = [\"k\"]\neasy = [\"l\"]\n").unwrap();
        assert_eq!(vec![parse_key("j").unwrap()], config.keys.again);
        assert_eq!(vec![parse_key("k").unwrap()], config.keys.good);
        assert_eq!(vec![parse_key("l").unwrap()], config.keys.easy);
        // Unmentioned actions keep their defaults.
        assert_eq!(Bindings::default().quit, config.keys.quit);
    }

    #[test]
    fn continue_is_a_valid_table_key() {
        let config = Config::from_toml("[keys]\ncontinue = [\"ctrl-n\"]\n").unwrap();
        assert_eq!(vec![parse_key("ctrl-n").unwrap()], config.keys.cont);
    }

    #[test]
    fn unknown_action_is_rejected() {
        assert!(Config::from_toml("[keys]\nfrobnicate = [\"x\"]\n").is_err());
    }

    #[test]
    fn unknown_section_is_rejected() {
        assert!(Config::from_toml("[keyz]\nagain = [\"x\"]\n").is_err());
    }

    #[test]
    fn bad_key_in_binding_is_rejected() {
        let err = Config::from_toml("[keys]\nagain = [\"jj\"]\n").unwrap_err();
        assert!(format!("{err:#}").contains("again"));
    }

    #[test]
    fn template_parses_to_defaults() {
        // As written (all settings commented out), the template overrides
        // nothing.
        let config = Config::from_toml(default_config_toml()).unwrap();
        assert_eq!(Config::default(), config);
    }

    #[test]
    fn template_commented_values_equal_defaults() {
        // Uncomment every `# key = ...` setting line and confirm the result is
        // still exactly the defaults — so the documented example values are
        // correct and valid TOML, not just inert (possibly rotted) comments.
        // `decks_dir` is excluded: its example "~/decks" is the *effective*
        // default but the struct default is `None` ("use ~/decks").
        let uncommented = default_config_toml()
            .lines()
            .map(|line| match line.trim_start().strip_prefix("# ") {
                Some(rest) if is_setting_line(rest) => rest,
                _ => line,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let config = Config::from_toml(&uncommented).unwrap();
        assert_eq!(Config::default(), config);
    }

    /// `true` if `s` looks like a `key = ...` assignment for a rebindable
    /// setting (lower-case/underscore key), other than `decks_dir`.
    fn is_setting_line(s: &str) -> bool {
        let Some((key, _)) = s.split_once('=') else {
            return false;
        };
        let key = key.trim();
        !key.is_empty()
            && key != "decks_dir"
            && key.chars().all(|c| c.is_ascii_lowercase() || c == '_')
    }

    #[test]
    fn ask_section_overrides_defaults() {
        let config = Config::from_toml(
            "[ask]\ncommand = \"my-claude\"\nmodel = \"haiku\"\ntimeout_secs = 30\n",
        )
        .unwrap();
        assert_eq!("my-claude", config.ask.command);
        assert_eq!(Some("haiku".to_string()), config.ask.model);
        assert_eq!(30, config.ask.timeout_secs);
        // Unmentioned fields keep their safe defaults.
        assert_eq!("dontAsk", config.ask.permission_mode);
        assert_eq!(vec!["WebFetch", "WebSearch"], config.ask.allowed_tools);
    }

    #[test]
    fn decks_dir_defaults_to_none_and_expands_tilde() {
        assert_eq!(None, Config::default().decks_dir);
        let config = Config::from_toml("decks_dir = \"~/my-decks\"\n").unwrap();
        let dir = config.decks_dir.unwrap();
        assert!(dir.ends_with("my-decks"));
        assert!(!dir.to_string_lossy().contains('~'));
    }

    #[test]
    fn ask_permission_and_tools_are_configurable() {
        let config = Config::from_toml(
            "[ask]\npermission_mode = \"bypassPermissions\"\n\
             allowed_tools = [\"WebFetch\", \"Read\"]\n",
        )
        .unwrap();
        assert_eq!("bypassPermissions", config.ask.permission_mode);
        assert_eq!(vec!["WebFetch", "Read"], config.ask.allowed_tools);
    }

    #[test]
    fn empty_model_means_cli_default() {
        let config = Config::from_toml("[ask]\nmodel = \"\"\n").unwrap();
        assert_eq!(None, config.ask.model);
    }

    #[test]
    fn unknown_ask_setting_is_rejected() {
        assert!(Config::from_toml("[ask]\ntemperature = 1.0\n").is_err());
    }

    #[test]
    fn browse_keys_default_to_vim_and_are_rebindable() {
        // Defaults: l/n/space next, h/p prev.
        let defaults = BrowseBindings::default();
        assert_eq!(parse_key("l").unwrap(), defaults.next[0]);
        assert_eq!(parse_key("h").unwrap(), defaults.prev[0]);

        let config = Config::from_toml("[browse]\nnext = [\"j\"]\nprev = [\"k\"]\n").unwrap();
        assert_eq!(vec![parse_key("j").unwrap()], config.browse.next);
        assert_eq!(vec![parse_key("k").unwrap()], config.browse.prev);
        // Unmentioned browse actions keep their defaults.
        assert_eq!(defaults.quit, config.browse.quit);
    }

    #[test]
    fn unknown_browse_setting_is_rejected() {
        assert!(Config::from_toml("[browse]\nfirst = [\"g\"]\n").is_err());
    }
}
