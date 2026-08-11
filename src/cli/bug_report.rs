use alix::{bug_report::BundleOptions, config::Config};
use anyhow::{Context, Result};

use crate::{BugReportArgs, profile};

pub(crate) fn bug_report_cmd(args: BugReportArgs) -> Result<()> {
    let config_path = match profile::resolve_default()? {
        Some(name) => {
            let path = profile::config_path_in(&profile::profiles_dir()?, &name);
            Some(path)
        }
        None => alix::config::default_config_path().filter(|path| path.is_file()),
    };
    let config = Config::load(config_path.as_deref())?;
    let root = config
        .decks_dir()
        .context("cannot determine the decks directory")?;
    let home = directories::BaseDirs::new().context("cannot determine the home directory")?;
    let log_paths = alix::log::log_paths()?;
    let tokens = config.serve.token.iter().cloned().collect::<Vec<String>>();
    let bundle = alix::bug_report::write_bundle_with(&BundleOptions {
        root: &root,
        out_dir: &args.out,
        config_path: config_path.as_deref(),
        log_paths: &log_paths,
        include_deck: args.include_deck.as_deref(),
        home: home.home_dir(),
        tokens: &tokens,
        now_ms: alix::time::now_ms(),
    })?;
    println!("wrote {}", bundle.path.display());
    println!("included {} files ({} bytes)", bundle.files, bundle.bytes);
    Ok(())
}
