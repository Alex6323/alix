import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync_client.dart' show SyncWriter;

void main() {
  group('deckIdForPath', () {
    final entries = [
      const SyncEntryState(
        entry: 'Biology',
        kind: 'workspace',
        decks: [
          SyncDeckState(
            deckId: 'deck-cells',
            path: 'decks/cells.md',
            unpushed: false,
          ),
          SyncDeckState(
            deckId: 'deck-organs',
            path: 'decks/organs.md',
            unpushed: false,
          ),
        ],
      ),
      const SyncEntryState(
        entry: 'Physics.md',
        kind: 'deck',
        decks: [
          SyncDeckState(
            deckId: 'deck-physics',
            path: 'Physics.md',
            unpushed: false,
          ),
        ],
      ),
    ];

    test('resolves a workspace member nested under its entry directory', () {
      expect(
        deckIdForPath(
          entries: entries,
          rootDir: '/support/paired/root-x',
          path: '/support/paired/root-x/Biology/decks/cells.md',
        ),
        'deck-cells',
      );
    });

    test('resolves a flattened loose deck directly under the root', () {
      expect(
        deckIdForPath(
          entries: entries,
          rootDir: '/support/paired/root-x',
          path: '/support/paired/root-x/Physics.md',
        ),
        'deck-physics',
      );
    });

    test('returns null for a path outside the root', () {
      expect(
        deckIdForPath(
          entries: entries,
          rootDir: '/support/paired/root-x',
          path: '/support/decks/cells.md',
        ),
        isNull,
      );
    });

    test('returns null for a path matching no known deck', () {
      expect(
        deckIdForPath(
          entries: entries,
          rootDir: '/support/paired/root-x',
          path: '/support/paired/root-x/Biology/decks/unknown.md',
        ),
        isNull,
      );
    });
  });

  group('SyncReport.summary', () {
    test('reports up to date when nothing happened', () {
      expect(const SyncReport().summary(), 'Synced: up to date');
    });

    test('names non-empty categories with their counts', () {
      final report = SyncReport(
        landed: const ['Biology/cells.md'],
        conflicts: const ['Biology/organs.md', 'Chemistry/atoms.md'],
        orphaned: const ['Old Deck'],
      );
      expect(report.summary(), 'Synced: 1 landed, 2 conflicts, 1 orphaned');
    });

    test('an error line replaces the category summary', () {
      final report = SyncReport(
        landed: const ['Biology/cells.md'],
        error: 'pairing expired',
      );
      expect(report.summary(), 'pairing expired');
    });
  });

  group('conflict wording', () {
    test('keep-phone names the desktop writer and time when known', () {
      final conflict = PairedConflictPush(
        desktopRevision: 9,
        desktopWriter: SyncWriter(
          device: 'desk-1',
          atMs: DateTime(2026, 1, 1, 9, 12).millisecondsSinceEpoch,
        ),
      );
      expect(
        conflictKeepPhoneLabel(conflict),
        "Keep the phone's progress (discards the desktop's, "
        'last written by desk-1 at 2026-01-01 09:12)',
      );
    });

    test('keep-phone omits the writer clause when none is known', () {
      const conflict = PairedConflictPush();
      expect(
        conflictKeepPhoneLabel(conflict),
        "Keep the phone's progress (discards the desktop's version)",
      );
    });

    test('a pull conflict names its own pulled-aside writer', () {
      final conflict = PairedConflictPull(
        pulledWriter: SyncWriter(
          device: 'desk-2',
          atMs: DateTime(2026, 1, 1, 8, 5).millisecondsSinceEpoch,
        ),
      );
      expect(
        conflictKeepPhoneLabel(conflict),
        "Keep the phone's progress (discards the desktop's, "
        'last written by desk-2 at 2026-01-01 08:05)',
      );
    });

    test('take-desktop wording never invents a review count', () {
      expect(
        conflictTakeDesktopLabel,
        "Take the desktop's (discards the phone's progress since the last sync)",
      );
    });
  });
}
