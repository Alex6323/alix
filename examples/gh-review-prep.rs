//! Example: understand a GitHub PR or issue before you act on it.
//!
//! Composes the alix *library* over an external source (here, a GitHub PR or
//! issue) to build a focused, ephemeral workspace of predict-and-verify traces
//! and fact decks — scoped to what the change or issue requires — which you then
//! drill and verify with `alix review` / `alix exam`. It only *reads* from
//! GitHub (`gh pr/issue view`, `gh pr diff`); it never touches the PR or issue.
//! This demonstrates the library's composability; it is not a GitHub-integration
//! feature of alix.
//!
//! Run it against the repo whose PR/issue you're reviewing. Point it there with
//! `--repo <path>` so you can `cargo run --example` straight from the alix repo:
//!
//!   cargo run --example gh-review-prep -- --repo /path/to/repo pr <n|url>
//!   cargo run --example gh-review-prep -- --repo /path/to/repo issue <n|url>
//!   cargo run --example gh-review-prep -- --repo /path/to/repo clean [pr|issue] <n>
//!
//! Without --repo it uses the current directory's repo — so from inside the repo
//! under review you can drop the flag (handy if you `cargo build --example` it
//! and symlink the binary onto your PATH as `gh-review-prep`).
//!
//! To ground the review in the right code, check out the branch first:
//!   … --branch <name>   check out <name> before exploring (works for a PR or issue)
//!   … --checkout        for a PR, check out its base — the branch it merges into
//!
//! Requires `gh` on PATH and a working `claude` CLI (alix shells out to it). The
//! workspace is disposable — retire it with `clean` after you merge or close.
//!
//! Env: ALIX_REVIEWS_DIR (default ~/reviews), ALIX_REVIEW_ICON=1 (draw an icon).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// Which kind of GitHub item we're understanding.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Pr,
    Issue,
}

impl Kind {
    /// The lowercase slug used in workspace and file names.
    fn slug(self) -> &'static str {
        match self {
            Kind::Pr => "pr",
            Kind::Issue => "issue",
        }
    }

    /// The human label used in the workspace title.
    fn label(self) -> &'static str {
        match self {
            Kind::Pr => "PR",
            Kind::Issue => "Issue",
        }
    }
}

/// A parsed command line.
#[derive(Debug, PartialEq, Eq)]
enum Cmd {
    Build { kind: Kind, id: String },
    Clean { kind: Kind, id: String },
    Help,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (opts, rest) = extract_opts(&args)?;
    match parse_args(&rest)? {
        Cmd::Help => {
            print_help();
            Ok(())
        }
        Cmd::Build { kind, id } => cmd_build(kind, &id, &opts),
        Cmd::Clean { kind, id } => cmd_clean(kind, &id, opts.repo.as_deref()),
    }
}

/// The optional flags this example accepts, pulled out of argv before the
/// positional `[pr|issue] <n>` is parsed.
struct Opts {
    /// `--repo <path>`: act on this repo instead of the current directory's.
    repo: Option<PathBuf>,
    /// `--branch <name>`: check out this branch first, so explore reads its code.
    branch: Option<String>,
    /// `--checkout`: for a PR, check out its base branch (the branch it merges into).
    checkout: bool,
}

/// Pull the optional flags (`--repo`, `--branch`, `--checkout`) out of the args —
/// they may appear anywhere — returning them plus the remaining args for
/// [`parse_args`].
fn extract_opts(args: &[String]) -> Result<(Opts, Vec<String>)> {
    let mut opts = Opts {
        repo: None,
        branch: None,
        checkout: false,
    };
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--repo" => {
                let path = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("--repo needs a path"))?;
                opts.repo = Some(PathBuf::from(path));
                i += 2;
            }
            "--branch" => {
                let name = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("--branch needs a name"))?;
                opts.branch = Some(name.clone());
                i += 2;
            }
            "--checkout" => {
                opts.checkout = true;
                i += 1;
            }
            _ => {
                rest.push(args[i].clone());
                i += 1;
            }
        }
    }
    Ok((opts, rest))
}

/// Parse argv (without the program name) into a [`Cmd`].
fn parse_args(args: &[String]) -> Result<Cmd> {
    match args.first().map(String::as_str) {
        None | Some("-h") | Some("--help") => Ok(Cmd::Help),
        Some("clean") => {
            let (kind, id) = parse_kind_id(&args[1..])?;
            Ok(Cmd::Clean { kind, id })
        }
        Some(_) => {
            let (kind, id) = parse_kind_id(args)?;
            Ok(Cmd::Build { kind, id })
        }
    }
}

/// Parse a `[pr|issue] <id>` / `<id>` tail into a kind (default PR) and id.
fn parse_kind_id(rest: &[String]) -> Result<(Kind, String)> {
    let (kind, id) = match rest.first().map(String::as_str) {
        Some("pr") => (Kind::Pr, rest.get(1)),
        Some("issue") => (Kind::Issue, rest.get(1)),
        Some(_) => (Kind::Pr, rest.first()),
        None => bail!("missing PR/issue number"),
    };
    let id = id.ok_or_else(|| anyhow::anyhow!("missing PR/issue number"))?;
    Ok((kind, id.clone()))
}

/// Normalize an id that may be a bare number or a full GitHub URL to its
/// trailing path segment (the number).
fn slug_id(id: &str) -> String {
    id.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(id)
        .to_string()
}

/// The transient context file dropped at the repo root (goal-referenced, then
/// removed after the build).
fn context_file_name(kind: Kind, id: &str) -> String {
    let ext = match kind {
        Kind::Pr => "diff",
        Kind::Issue => "md",
    };
    format!(".alix-review-{}-{id}.{ext}", kind.slug())
}

/// `<reviews_dir>/<repo>-<kind>-<id>`.
fn workspace_path(reviews_dir: &Path, repo: &str, kind: Kind, id: &str) -> PathBuf {
    reviews_dir.join(format!("{repo}-{}-{id}", kind.slug()))
}

/// Whether to warn about a stale/mismatched checkout: true when more than half
/// the changed files are missing from the working tree.
fn most_files_missing(total: usize, missing: usize) -> bool {
    total > 0 && missing * 2 > total
}

/// The learning goal that scopes `explore` to the change/issue and its context.
fn build_goal(kind: Kind, id: &str, title: &str, file_name: &str) -> String {
    match kind {
        Kind::Pr => format!(
            "Understand PR #{id} (\"{title}\"), whose full description and unified \
             diff are in ./{file_name}. Read that diff to see exactly what changed, \
             then use the surrounding repository as context. Cover the functions and \
             types the diff modifies and their blast radius across this repo: who \
             calls them, what depends on them, and which invariants or contracts the \
             change affects. Exclude code unrelated to this change."
        ),
        Kind::Issue => format!(
            "Understand issue #{id} (\"{title}\"), whose full text and discussion are \
             in ./{file_name}. Work out what is actually being asked or reported (the \
             author's description may be incomplete), which part of this repository it \
             concerns, the concepts needed to reason about it, and — if it is a bug — \
             how it could arise. Use the repository for context. Exclude code \
             unrelated to the issue."
        ),
    }
}

/// Run `gh` with args in `dir`, returning trimmed stdout; error on non-zero exit.
fn run_gh(dir: &Path, args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("gh")
        .current_dir(dir)
        .args(args)
        .output()
        .context("failed to run gh — is the GitHub CLI installed and on PATH?")?;
    if !out.status.success() {
        bail!(
            "gh {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The git work-tree root the tool acts on: `--repo <path>` if given, else the
/// current directory's repo.
fn resolve_repo(repo: Option<&Path>) -> Result<PathBuf> {
    let mut cmd = std::process::Command::new("git");
    if let Some(dir) = repo {
        cmd.current_dir(dir);
    }
    let out = cmd
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("failed to run git")?;
    if !out.status.success() {
        match repo {
            Some(dir) => bail!("{} is not inside a git work tree", dir.display()),
            None => {
                bail!("run this from inside the repo the item belongs to, or pass --repo <path>")
            }
        }
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

/// Check out `name` in `root` so the working tree reflects that branch before we
/// explore it. `git checkout` refuses on a dirty tree or an unknown ref; we
/// surface that error and stop — never forced, never clobbering your work.
fn checkout_branch(root: &Path, name: &str) -> Result<()> {
    let out = std::process::Command::new("git")
        .current_dir(root)
        .args(["checkout", name])
        .output()
        .context("failed to run git checkout")?;
    if !out.status.success() {
        bail!(
            "could not check out `{name}`: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    println!("checked out `{name}`");
    Ok(())
}

/// The base branch a PR targets — the branch it wants to merge into.
fn pr_base_branch(root: &Path, id: &str) -> Result<String> {
    let base = run_gh(
        root,
        &[
            "pr",
            "view",
            id,
            "--json",
            "baseRefName",
            "-q",
            ".baseRefName",
        ],
    )?;
    if base.is_empty() {
        bail!("could not determine the base branch of PR {id}");
    }
    Ok(base)
}

/// The repo's GitHub name, else the work-tree directory name.
fn repo_slug(root: &Path) -> String {
    if let Ok(name) = run_gh(root, &["repo", "view", "--json", "name", "-q", ".name"])
        && !name.is_empty()
    {
        return name;
    }
    root.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("repo")
        .to_string()
}

/// Where review workspaces live (`$ALIX_REVIEWS_DIR`, else `~/reviews`).
fn reviews_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ALIX_REVIEWS_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("reviews")
}

/// Fetch the PR/issue text (and, for a PR, the diff) from GitHub, write it to
/// `dest`, and return the item's title.
fn fetch_source(kind: Kind, id: &str, dest: &Path, root: &Path) -> Result<String> {
    let title = run_gh(
        root,
        &[kind.slug(), "view", id, "--json", "title", "-q", ".title"],
    )?;
    let body = run_gh(
        root,
        &[kind.slug(), "view", id, "--json", "body", "-q", ".body"],
    )?;
    let mut content = format!("# {} #{id}: {title}\n\n{body}\n", kind.label());
    match kind {
        Kind::Pr => {
            let diff = run_gh(root, &["pr", "diff", id])?;
            content.push_str(&format!("\n## Diff\n\n{diff}\n"));
        }
        Kind::Issue => {
            let comments = run_gh(
                root,
                &[
                    "issue",
                    "view",
                    id,
                    "--json",
                    "comments",
                    "-q",
                    r#".comments[] | "\(.author.login):\n\(.body)\n""#,
                ],
            )?;
            if !comments.is_empty() {
                content.push_str(&format!("\n## Comments\n\n{comments}\n"));
            }
        }
    }
    std::fs::write(dest, content).with_context(|| format!("failed to write {}", dest.display()))?;
    Ok(title)
}

/// For a PR, warn (non-fatally) if most of its changed files are absent from the
/// working tree — a sign you're in the wrong repo or on a branch without the
/// change, which weakens the blast-radius context. Best-effort: a failed probe
/// is silently skipped, and a warning never blocks the build.
fn warn_if_source_stale(id: &str, display: &str, root: &Path) {
    let Ok(files) = run_gh(
        root,
        &["pr", "view", id, "--json", "files", "-q", ".files[].path"],
    ) else {
        return;
    };
    let paths: Vec<&str> = files
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let missing = paths
        .iter()
        .filter(|path| !root.join(path).exists())
        .count();
    if most_files_missing(paths.len(), missing) {
        eprintln!(
            "warning: {missing} of {} changed files from PR #{display} aren't in your working \
             tree — you may be in the wrong repo or on a branch without this change; consider \
             `gh pr checkout {display}` for fuller context",
            paths.len()
        );
    }
}

/// The five library calls that turn (source, goal) into a filled, snapshotted
/// workspace — the same sequence `alix explore --build` runs internally.
fn build_workspace(
    root: &Path,
    ws: &Path,
    title: &str,
    goal: &str,
) -> Result<alix::explore::Materialized> {
    let config = alix::config::Config::load(None).context("failed to load alix config")?;
    let source_text = root.to_string_lossy();
    let source: &str = &source_text;
    let (plan, filled) = alix::explore::explore_and_fill(source, goal, &config.trace, &config.ask)
        .context("explore/fill failed")?;
    let report = alix::explore::materialize(&plan, ws, goal, Some(title), source, Some(&filled))
        .context("failed to materialize the workspace")?;
    if let Err(e) = alix::explore::snapshot_workspace(&report.dir) {
        eprintln!("warning: source snapshot failed: {e}");
    }
    if std::env::var("ALIX_REVIEW_ICON").as_deref() == Ok("1")
        && let Err(e) = alix::icon::generate(&report.dir, None, &config.ask)
    {
        eprintln!("warning: icon generation failed: {e}");
    }
    Ok(report)
}

fn cmd_build(kind: Kind, id: &str, opts: &Opts) -> Result<()> {
    let root = resolve_repo(opts.repo.as_deref())?;
    let slug = slug_id(id);
    let repo_name = repo_slug(&root);
    let ws = workspace_path(&reviews_dir(), &repo_name, kind, &slug);
    if ws.exists() {
        let retire = match &opts.repo {
            Some(p) => format!(
                "gh-review-prep --repo {} clean {} {slug}",
                p.display(),
                kind.slug()
            ),
            None => format!("gh-review-prep clean {} {slug}", kind.slug()),
        };
        bail!(
            "review workspace already exists: {} (retire it with: {retire})",
            ws.display()
        );
    }
    // Optionally put the working tree on the branch to review, so explore reads
    // the right code. An explicit --branch wins; --checkout on a PR falls back to
    // its base (the branch it merges into).
    if let Some(name) = &opts.branch {
        checkout_branch(&root, name)?;
    } else if opts.checkout {
        match kind {
            Kind::Pr => {
                let base = pr_base_branch(&root, id)?;
                checkout_branch(&root, &base)?;
            }
            Kind::Issue => {
                bail!("an issue has no base branch to check out — name one with --branch <name>")
            }
        }
    }
    if kind == Kind::Pr {
        warn_if_source_stale(id, &slug, &root);
    }
    let file_name = context_file_name(kind, &slug);
    let file = root.join(&file_name);
    let item_title = fetch_source(kind, id, &file, &root)?;
    let goal = build_goal(kind, &slug, &item_title, &file_name);
    let title = format!("Review: {} #{slug} — {item_title}", kind.label());
    let result = build_workspace(&root, &ws, &title, &goal);
    let _ = std::fs::remove_file(&file); // transient context; the workspace grounds on the repo
    let report = result?;
    print_next_steps(&ws, &report)
}

fn cmd_clean(kind: Kind, id: &str, repo: Option<&Path>) -> Result<()> {
    let root = resolve_repo(repo)?;
    let slug = slug_id(id);
    let repo_name = repo_slug(&root);
    let ws = workspace_path(&reviews_dir(), &repo_name, kind, &slug);
    if !ws.is_dir() {
        bail!("no review workspace at {}", ws.display());
    }
    std::fs::remove_dir_all(&ws).with_context(|| format!("failed to remove {}", ws.display()))?;
    let _ = std::fs::remove_file(root.join(context_file_name(kind, &slug))); // leftover, if any
    println!("retired {}", ws.display());
    Ok(())
}

fn print_next_steps(ws: &Path, report: &alix::explore::Materialized) -> Result<()> {
    println!(
        "\nReady — {} decks, {} traces in: {}",
        report.decks,
        report.traces,
        ws.display()
    );
    for entry in std::fs::read_dir(ws).with_context(|| format!("reading {}", ws.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("txt") {
            println!("  {}", path.display());
        }
    }
    println!("\nDrill, then verify before you act:");
    println!("  alix review \"{}\"", ws.display());
    println!("  alix exam   \"{}\"/<deck>.txt", ws.display());
    Ok(())
}

fn print_help() {
    println!(
        "gh-review-prep — build an ephemeral alix workspace to understand a GitHub PR or issue\n\
         \n\
         USAGE:\n\
         \x20 gh-review-prep [--repo <path>] [--branch <name>|--checkout] [pr] <n|url>   build a PR workspace\n\
         \x20 gh-review-prep [--repo <path>] [--branch <name>] issue <n|url>             build an issue workspace\n\
         \x20 gh-review-prep [--repo <path>] clean [pr|issue] <n>                        retire a workspace\n\
         \n\
         --repo <path>    target that repo (default: the current directory's repo)\n\
         --branch <name>  check out <name> first, so explore reads that branch's code\n\
         --checkout       for a PR, check out its base — the branch it merges into\n\
         Via cargo from the alix repo: cargo run --example gh-review-prep -- --repo <path> pr <n>\n\
         ENV: ALIX_REVIEWS_DIR (default ~/reviews), ALIX_REVIEW_ICON=1 (draw an icon)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parse_args_defaults_bare_number_to_pr_build() {
        assert_eq!(
            parse_args(&s(&["123"])).unwrap(),
            Cmd::Build {
                kind: Kind::Pr,
                id: "123".into()
            }
        );
    }

    #[test]
    fn parse_args_explicit_issue_build() {
        assert_eq!(
            parse_args(&s(&["issue", "7"])).unwrap(),
            Cmd::Build {
                kind: Kind::Issue,
                id: "7".into()
            }
        );
    }

    #[test]
    fn parse_args_explicit_pr_build() {
        assert_eq!(
            parse_args(&s(&["pr", "9"])).unwrap(),
            Cmd::Build {
                kind: Kind::Pr,
                id: "9".into()
            }
        );
    }

    #[test]
    fn parse_args_clean_defaults_to_pr() {
        assert_eq!(
            parse_args(&s(&["clean", "4"])).unwrap(),
            Cmd::Clean {
                kind: Kind::Pr,
                id: "4".into()
            }
        );
    }

    #[test]
    fn parse_args_clean_issue() {
        assert_eq!(
            parse_args(&s(&["clean", "issue", "4"])).unwrap(),
            Cmd::Clean {
                kind: Kind::Issue,
                id: "4".into()
            }
        );
    }

    #[test]
    fn parse_args_empty_is_help() {
        assert_eq!(parse_args(&s(&[])).unwrap(), Cmd::Help);
    }

    #[test]
    fn parse_args_missing_number_errors() {
        assert!(parse_args(&s(&["issue"])).is_err());
    }

    #[test]
    fn slug_id_bare_number_unchanged() {
        assert_eq!(slug_id("123"), "123");
    }

    #[test]
    fn slug_id_extracts_number_from_url() {
        assert_eq!(slug_id("https://github.com/owner/repo/pull/123"), "123");
        assert_eq!(slug_id("https://github.com/owner/repo/issues/45/"), "45");
    }

    #[test]
    fn context_file_name_uses_kind_and_ext() {
        assert_eq!(context_file_name(Kind::Pr, "12"), ".alix-review-pr-12.diff");
        assert_eq!(
            context_file_name(Kind::Issue, "12"),
            ".alix-review-issue-12.md"
        );
    }

    #[test]
    fn workspace_path_joins_repo_kind_id() {
        let p = workspace_path(Path::new("/r"), "alix", Kind::Pr, "8");
        assert_eq!(p, PathBuf::from("/r/alix-pr-8"));
    }

    #[test]
    fn build_goal_pr_mentions_change_blast_radius_and_file() {
        let g = build_goal(Kind::Pr, "3", "Fix login", ".alix-review-pr-3.diff");
        assert!(g.contains("PR #3"));
        assert!(g.contains("blast radius"));
        assert!(g.contains(".alix-review-pr-3.diff"));
    }

    #[test]
    fn build_goal_issue_mentions_asked_and_file() {
        let g = build_goal(Kind::Issue, "5", "Crash on save", ".alix-review-issue-5.md");
        assert!(g.contains("issue #5"));
        assert!(g.contains("actually being asked"));
        assert!(g.contains(".alix-review-issue-5.md"));
    }

    #[test]
    fn most_files_missing_true_when_majority_absent() {
        assert!(most_files_missing(4, 3));
        assert!(most_files_missing(2, 2));
        assert!(most_files_missing(5, 3));
    }

    #[test]
    fn most_files_missing_false_at_half_or_below_or_empty() {
        assert!(!most_files_missing(4, 2)); // exactly half → no warn
        assert!(!most_files_missing(5, 2));
        assert!(!most_files_missing(0, 0)); // no files → no signal
    }

    #[test]
    fn extract_opts_pulls_repo_and_leaves_rest() {
        let (opts, rest) = extract_opts(&s(&["--repo", "/x", "pr", "5"])).unwrap();
        assert_eq!(opts.repo, Some(PathBuf::from("/x")));
        assert_eq!(rest, s(&["pr", "5"]));
    }

    #[test]
    fn extract_opts_none_when_absent() {
        let (opts, rest) = extract_opts(&s(&["pr", "5"])).unwrap();
        assert_eq!(opts.repo, None);
        assert_eq!(opts.branch, None);
        assert!(!opts.checkout);
        assert_eq!(rest, s(&["pr", "5"]));
    }

    #[test]
    fn extract_opts_flags_can_appear_anywhere() {
        let (opts, rest) = extract_opts(&s(&["issue", "--repo", "/x", "7"])).unwrap();
        assert_eq!(opts.repo, Some(PathBuf::from("/x")));
        assert_eq!(rest, s(&["issue", "7"]));
    }

    #[test]
    fn extract_opts_pulls_branch_and_checkout() {
        let (opts, rest) =
            extract_opts(&s(&["--branch", "feature-x", "--checkout", "pr", "5"])).unwrap();
        assert_eq!(opts.branch.as_deref(), Some("feature-x"));
        assert!(opts.checkout);
        assert_eq!(rest, s(&["pr", "5"]));
    }

    #[test]
    fn extract_opts_errors_without_a_value() {
        assert!(extract_opts(&s(&["--repo"])).is_err());
        assert!(extract_opts(&s(&["--branch"])).is_err());
    }
}
