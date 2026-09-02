#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    // Forces RUST_BACKTRACE=1, so anyhow appends a "Stack backtrace:" tail
    // to every bridged error's Debug form; the Dart side strips it for
    // display (bridge_error.dart) so panic diagnostics stay captured
    // without polluting user-facing error text.
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

    #[test]
    fn init_app_arms_captured_panic_backtraces_for_the_bridge() {
        // Backtrace::capture() freezes its env decision process-wide at the
        // first capture, so a shared test process cannot assert Captured
        // here: an earlier test's anyhow error locks in the unset-env
        // choice. The armed variable is the mechanism under test; in the
        // app init_app runs before any capture can happen.
        init_app();
        assert_eq!(std::env::var("RUST_BACKTRACE").as_deref(), Ok("1"));
        let error = match flutter_rust_bridge::PanicBacktrace::catch_unwind(|| {
            panic!("panic-backtrace probe")
        }) {
            Ok(()) => panic!("the probe panic was not caught"),
            Err(error) => error,
        };
        assert!(
            error.backtrace.is_some(),
            "the frb panic hook must attach a backtrace"
        );
    }
}
