#![no_main]

use std::{fs, path::PathBuf, sync::OnceLock};

use alix::stamp::{StampOutcome, stamp_deck};
use libfuzzer_sys::fuzz_target;

static DECK_PATH: OnceLock<PathBuf> = OnceLock::new();

fn deck_path() -> &'static PathBuf {
    DECK_PATH.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("alix-stamp-fuzz-{}", std::process::id()));
        fs::create_dir_all(&root).expect("create the process-local fuzz directory");
        root.join("deck.md")
    })
}

fuzz_target!(|input: &[u8]| {
    let path = deck_path();
    fs::write(path, input).expect("reset the process-local deck");

    match stamp_deck(path) {
        Err(_) => {
            assert_eq!(
                fs::read(path).expect("read a rejected deck"),
                input,
                "stamp_deck changed bytes while rejecting its input"
            );
        }
        Ok(_) => {
            let once = fs::read(path).expect("read a stamped deck");
            let second = stamp_deck(path).expect("a stamped deck must remain valid");
            assert_eq!(
                second,
                StampOutcome::default(),
                "a second stamp minted more identifiers"
            );
            assert_eq!(
                fs::read(path).expect("read the twice-stamped deck"),
                once,
                "a second stamp changed the deck bytes"
            );
        }
    }
});
