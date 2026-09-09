use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub candidates_classified: u64,
    pub manifest_reads: u64,
    pub decks_loaded: u64,
    pub prerequisite_loads: u64,
    pub id_scans: u64,
    pub diagram_geometry_reads: u64,
    pub store_documents_read: u64,
    pub augment_documents_read: u64,
    pub canonicalize_calls: u64,
    pub fingerprint_hashes: u64,
    pub sidecar_reads: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Counter {
    CandidatesClassified,
    ManifestReads,
    DecksLoaded,
    PrerequisiteLoads,
    IdScans,
    DiagramGeometryReads,
    StoreDocumentsRead,
    AugmentDocumentsRead,
    CanonicalizeCalls,
    FingerprintHashes,
    SidecarReads,
}

impl Counts {
    fn bump(&mut self, counter: Counter) {
        let slot = match counter {
            Counter::CandidatesClassified => &mut self.candidates_classified,
            Counter::ManifestReads => &mut self.manifest_reads,
            Counter::DecksLoaded => &mut self.decks_loaded,
            Counter::PrerequisiteLoads => &mut self.prerequisite_loads,
            Counter::IdScans => &mut self.id_scans,
            Counter::DiagramGeometryReads => &mut self.diagram_geometry_reads,
            Counter::StoreDocumentsRead => &mut self.store_documents_read,
            Counter::AugmentDocumentsRead => &mut self.augment_documents_read,
            Counter::CanonicalizeCalls => &mut self.canonicalize_calls,
            Counter::FingerprintHashes => &mut self.fingerprint_hashes,
            Counter::SidecarReads => &mut self.sidecar_reads,
        };
        *slot += 1;
    }

    pub fn get(&self, counter: Counter) -> u64 {
        match counter {
            Counter::CandidatesClassified => self.candidates_classified,
            Counter::ManifestReads => self.manifest_reads,
            Counter::DecksLoaded => self.decks_loaded,
            Counter::PrerequisiteLoads => self.prerequisite_loads,
            Counter::IdScans => self.id_scans,
            Counter::DiagramGeometryReads => self.diagram_geometry_reads,
            Counter::StoreDocumentsRead => self.store_documents_read,
            Counter::AugmentDocumentsRead => self.augment_documents_read,
            Counter::CanonicalizeCalls => self.canonicalize_calls,
            Counter::FingerprintHashes => self.fingerprint_hashes,
            Counter::SidecarReads => self.sidecar_reads,
        }
    }
}

pub const ALL_COUNTERS: [Counter; 11] = [
    Counter::CandidatesClassified,
    Counter::ManifestReads,
    Counter::DecksLoaded,
    Counter::PrerequisiteLoads,
    Counter::IdScans,
    Counter::DiagramGeometryReads,
    Counter::StoreDocumentsRead,
    Counter::AugmentDocumentsRead,
    Counter::CanonicalizeCalls,
    Counter::FingerprintHashes,
    Counter::SidecarReads,
];

thread_local! {
    static ACTIVE: Cell<Option<Counts>> = const { Cell::new(None) };
}

pub fn collect<T>(f: impl FnOnce() -> T) -> (T, Counts) {
    struct Restore {
        outer: Option<Counts>,
        armed: bool,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            if self.armed {
                ACTIVE.with(|active| active.set(self.outer));
            }
        }
    }
    let mut guard = Restore {
        outer: ACTIVE.with(|active| active.replace(Some(Counts::default()))),
        armed: true,
    };
    let value = f();
    guard.armed = false;
    let counts = ACTIVE
        .with(|active| active.replace(guard.outer))
        .unwrap_or_default();
    (value, counts)
}

pub(crate) fn hit(counter: Counter) {
    ACTIVE.with(|active| {
        let Some(mut counts) = active.take() else {
            return;
        };
        counts.bump(counter);
        active.set(Some(counts));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hit_outside_any_collection_is_dropped_and_leaves_nothing_behind() {
        hit(Counter::DecksLoaded);
        let ((), counts) = collect(|| {});
        assert_eq!(
            counts,
            Counts::default(),
            "a stray hit must not leak into a later collection"
        );
    }

    #[test]
    fn every_counter_is_collected_once_per_hit_and_reset_per_collection() {
        let ((), counts) = collect(|| {
            for counter in ALL_COUNTERS {
                hit(counter);
                hit(counter);
            }
        });
        for counter in ALL_COUNTERS {
            assert_eq!(counts.get(counter), 2, "{counter:?} after two hits");
        }
        let ((), again) = collect(|| hit(Counter::IdScans));
        assert_eq!(again.id_scans, 1, "id_scans in a fresh collection");
        assert_eq!(
            again.decks_loaded, 0,
            "decks_loaded must reset between collections"
        );
    }

    #[test]
    fn a_nested_collection_keeps_its_hits_and_restores_the_outer_one() {
        let ((), outer) = collect(|| {
            hit(Counter::DecksLoaded);
            let ((), inner) = collect(|| hit(Counter::ManifestReads));
            assert_eq!(inner.manifest_reads, 1, "inner sees its own hit");
            assert_eq!(inner.decks_loaded, 0, "inner does not see the outer hit");
            hit(Counter::DecksLoaded);
        });
        assert_eq!(
            outer.decks_loaded, 2,
            "outer counts before and after the nested collection"
        );
        assert_eq!(outer.manifest_reads, 0, "inner hits do not flow outward");
    }

    #[test]
    fn a_panic_inside_a_collection_restores_the_outer_collector() {
        let ((), outer) = collect(|| {
            let caught = std::panic::catch_unwind(|| {
                collect(|| {
                    hit(Counter::IdScans);
                    panic!("inner failure");
                })
            });
            assert!(caught.is_err(), "the inner panic propagates");
            hit(Counter::DecksLoaded);
        });
        assert_eq!(
            outer.decks_loaded, 1,
            "the outer collector is active again after the panic"
        );
        assert_eq!(outer.id_scans, 0, "the aborted inner hits are discarded");
    }

    #[test]
    fn the_value_of_the_closure_comes_back_with_the_counts() {
        let (value, counts) = collect(|| 42);
        assert_eq!(value, 42);
        assert_eq!(counts, Counts::default());
    }
}
