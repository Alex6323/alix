#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    // Not setup_default_user_utils: that forces RUST_BACKTRACE=1, which
    // anyhow captures and Debug-prints into every bridged error string the
    // UI shows verbatim. Only its panic-hook half is kept.
    flutter_rust_bridge::PanicBacktrace::setup();
}

#[flutter_rust_bridge::frb(sync)]
pub fn core_version() -> String {
    alix::VERSION.to_string()
}

pub fn stamp_deck(path: String) -> anyhow::Result<()> {
    alix::stamp::stamp_deck(std::path::Path::new(&path))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_version_is_a_three_part_semver() {
        let version = core_version();
        assert_eq!(version.split('.').count(), 3, "{version}");
        assert!(
            version.split('.').all(|part| part.parse::<u32>().is_ok()),
            "{version}"
        );
    }
}
