use std::fmt;

/// What kind of thing broke, which fixes the one remedy sentence a failure
/// carries. The wire names are API contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncFailureClass {
    /// Alix's own sync state under the served folder cannot be read or
    /// written: the root identity file, a progress file, the staging folder.
    DamagedSyncState,
    /// A file the learner owns inside a served entry cannot be read or
    /// cannot travel: a deck, its personal sidecar, a media file, a link.
    UnreadableDeckFile,
    /// The request itself is one this server cannot serve.
    MalformedRequest,
}

impl SyncFailureClass {
    pub fn wire(self) -> &'static str {
        match self {
            Self::DamagedSyncState => "damaged-sync-state",
            Self::UnreadableDeckFile => "unreadable-deck-file",
            Self::MalformedRequest => "malformed-request",
        }
    }

    pub fn remedy(self) -> &'static str {
        match self {
            Self::DamagedSyncState => "Run `alix doctor` on the computer.",
            Self::UnreadableDeckFile => "Check the file on the computer, then run `alix doctor`.",
            Self::MalformedRequest => "Update both apps, then file a bug report.",
        }
    }
}

/// A paired-sync failure whose message names no path and no user-authored
/// file name: alix's own fixed names stay literal, a deck is named by its
/// minted id, and everything else is named by its kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncFailure {
    pub class: SyncFailureClass,
    pub message: String,
}

impl SyncFailure {
    pub fn new(class: SyncFailureClass, message: impl Into<String>) -> Self {
        Self {
            class,
            message: message.into(),
        }
    }

    pub fn damaged(message: impl Into<String>) -> Self {
        Self::new(SyncFailureClass::DamagedSyncState, message)
    }

    pub fn unreadable(message: impl Into<String>) -> Self {
        Self::new(SyncFailureClass::UnreadableDeckFile, message)
    }

    pub fn malformed(message: impl Into<String>) -> Self {
        Self::new(SyncFailureClass::MalformedRequest, message)
    }

    pub fn remedy(&self) -> &'static str {
        self.class.remedy()
    }
}

impl fmt::Display for SyncFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\n{}", self.message, self.remedy())
    }
}

impl std::error::Error for SyncFailure {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_class_has_a_wire_name_and_a_remedy_sentence() {
        for class in [
            SyncFailureClass::DamagedSyncState,
            SyncFailureClass::UnreadableDeckFile,
            SyncFailureClass::MalformedRequest,
        ] {
            assert!(
                class
                    .wire()
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '-'),
                "wire name is kebab-case: {}",
                class.wire()
            );
            assert!(
                class.remedy().ends_with('.'),
                "remedy is a sentence: {}",
                class.remedy()
            );
        }
    }

    #[test]
    fn display_puts_the_remedy_on_its_own_second_line() {
        let failure = SyncFailure::damaged("cannot read .alix/sync.toml");
        assert_eq!(
            "cannot read .alix/sync.toml\nRun `alix doctor` on the computer.",
            failure.to_string()
        );
    }
}
