#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
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
