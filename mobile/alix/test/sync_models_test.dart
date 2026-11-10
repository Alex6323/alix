import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync_client.dart' show SyncWriter;

void main() {
  group('deckIdForPath', () {
    final entries = [
      const SyncEntryState(
        entry: 'Biology',
        kind: 'workspace',
        fingerprint: '0000000000001',
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
        fingerprint: '0000000000001',
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
    final morning = DateTime(2026, 1, 1, 9, 12).millisecondsSinceEpoch;
    final evening = DateTime(2026, 1, 1, 19, 28).millisecondsSinceEpoch;
    SyncPendingConflict pending({
      PairedConflict conflict = const PairedConflictPull(),
      int phoneCards = 0,
      int? phoneAtMs,
    }) =>
        SyncPendingConflict(
          deckId: 'deck-1',
          label: 'German/Verbs.md',
          conflict: conflict,
          phoneAtMs: phoneAtMs,
          phoneCards: phoneCards,
        );

    test('both rows keep a side and name what they keep', () {
      final conflict = pending(
        conflict: PairedConflictPull(
          pulledWriter: SyncWriter(device: 'desk-1', atMs: evening),
        ),
        phoneCards: 18,
        phoneAtMs: morning,
      );
      final phone = conflictKeepPhoneWording(conflict);
      final desktop = conflictKeepDesktopWording(conflict);
      expect(phone.title, "Keep the phone's progress", reason: 'phone title');
      expect(desktop.title, "Keep the desktop's progress", reason: 'desktop title');
      expect(phone.subtitle, '18 cards with progress, last saved 2026-01-01 09:12',
          reason: 'phone subtitle');
      expect(desktop.subtitle, 'from desk-1, last saved 2026-01-01 19:28',
          reason: 'desktop subtitle');
    });

    test('the later save is marked newer, on either side', () {
      final desktopLater = pending(
        conflict: PairedConflictPush(
          desktopWriter: SyncWriter(device: 'desk-1', atMs: evening),
        ),
        phoneAtMs: morning,
      );
      expect(
        (conflictKeepPhoneWording(desktopLater).newer,
            conflictKeepDesktopWording(desktopLater).newer),
        (false, true),
        reason: 'desktop saved later',
      );
      final phoneLater = pending(
        conflict: PairedConflictPush(
          desktopWriter: SyncWriter(device: 'desk-1', atMs: morning),
        ),
        phoneAtMs: evening,
      );
      expect(
        (conflictKeepPhoneWording(phoneLater).newer,
            conflictKeepDesktopWording(phoneLater).newer),
        (true, false),
        reason: 'phone saved later',
      );
    });

    test('no side is marked newer when either time is unknown', () {
      final conflict = pending(phoneAtMs: morning);
      expect(
        (conflictKeepPhoneWording(conflict).newer,
            conflictKeepDesktopWording(conflict).newer),
        (false, false),
      );
    });

    test('a phone side without card progress says so', () {
      expect(
        conflictKeepPhoneWording(pending(phoneAtMs: morning)).subtitle,
        'no card progress, last saved 2026-01-01 09:12',
      );
    });

    test('one card is singular and an unknown time is left out', () {
      expect(conflictKeepPhoneWording(pending(phoneCards: 1)).subtitle,
          '1 card with progress');
    });

    test('an unknown desktop writer still names the side', () {
      expect(
        conflictKeepDesktopWording(pending(conflict: const PairedConflictPush()))
            .subtitle,
        'from the desktop',
      );
    });
  });
}
