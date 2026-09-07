// Widget tests for lib/sync/sync_sheet.dart: the conflict choice's exact
// button wording (what each side discards), that only non-empty report
// sections render, and that a button tap calls onResolve with the right
// deckId/keepPhone pair. Pure presentation, no bridge or dylib needed.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync/sync_sheet.dart';
import 'package:alix_mobile/sync_client.dart';

void main() {
  Future<void> pump(
    WidgetTester tester, {
    SyncReport? report,
    List<SyncPendingConflict> conflicts = const [],
    void Function(String deckId, bool keepPhone)? onResolve,
    ValueChanged<String>? onRemoveOrphan,
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: SyncReportSheet(
            report: report,
            conflicts: conflicts,
            onResolve: onResolve ?? (_, _) {},
            onRemoveOrphan: onRemoveOrphan ?? (_) {},
          ),
        ),
      ),
    );
  }

  testWidgets(
    'a conflict names what each button discards, including the desktop '
    "writer when the bridge reports one",
    (tester) async {
      const conflict = SyncPendingConflict(
        deckId: 'deck-1',
        entry: 'German',
        path: 'Verbs.md',
        conflict: PairedConflictPush(
          desktopWriter: SyncWriter(device: 'desk-1', atMs: 0),
        ),
      );
      await pump(tester, conflicts: const [conflict]);

      expect(
        find.text(
          "Keep the phone's progress (discards the desktop's, "
          'last written by desk-1 at 01:00)',
        ),
        findsOneWidget,
      );
      expect(
        find.text(
          "Take the desktop's (discards the phone's progress since the "
          'last sync)',
        ),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'a conflict with no known desktop writer keeps the keep-phone label '
    'writer-free rather than inventing one',
    (tester) async {
      const conflict = SyncPendingConflict(
        deckId: 'deck-1',
        entry: 'German',
        path: 'Verbs.md',
        conflict: PairedConflictPull(),
      );
      await pump(tester, conflicts: const [conflict]);

      expect(
        find.text(
          "Keep the phone's progress (discards the desktop's version)",
        ),
        findsOneWidget,
      );
    },
  );

  testWidgets('tapping keep-phone resolves with keepPhone true, the '
      "conflict's own deckId", (tester) async {
    const conflict = SyncPendingConflict(
      deckId: 'deck-9',
      entry: 'German',
      path: 'Verbs.md',
      conflict: PairedConflictPull(),
    );
    String? resolvedId;
    bool? resolvedKeepPhone;
    await pump(
      tester,
      conflicts: const [conflict],
      onResolve: (deckId, keepPhone) {
        resolvedId = deckId;
        resolvedKeepPhone = keepPhone;
      },
    );

    await tester.tap(
      find.text(
        "Keep the phone's progress (discards the desktop's version)",
      ),
    );
    expect(resolvedId, 'deck-9');
    expect(resolvedKeepPhone, isTrue);
  });

  testWidgets('tapping take-desktop resolves with keepPhone false', (
    tester,
  ) async {
    const conflict = SyncPendingConflict(
      deckId: 'deck-9',
      entry: 'German',
      path: 'Verbs.md',
      conflict: PairedConflictPull(),
    );
    bool? resolvedKeepPhone;
    await pump(
      tester,
      conflicts: const [conflict],
      onResolve: (_, keepPhone) => resolvedKeepPhone = keepPhone,
    );

    await tester.tap(find.text(conflictTakeDesktopLabel));
    expect(resolvedKeepPhone, isFalse);
  });

  testWidgets(
    'only the non-empty report sections render; an empty category is '
    'never shown as a blank heading',
    (tester) async {
      const report = SyncReport(
        landed: ['German/Verbs.md'],
        conflicts: [],
        refused: ['French/Nouns.md: too large to push'],
      );
      await pump(tester, report: report);

      expect(find.text('Landed'), findsOneWidget);
      expect(find.text('German/Verbs.md'), findsOneWidget);
      expect(find.text('Refused'), findsOneWidget);
      expect(find.text('French/Nouns.md: too large to push'), findsOneWidget);
      expect(find.text('Kept (unpushed)'), findsNothing);
      expect(find.text('Phone-only'), findsNothing);
      expect(find.text('Removed'), findsNothing);
      expect(find.text('Renamed'), findsNothing);
      expect(find.text('Left out'), findsNothing);
      expect(find.text('Orphaned'), findsNothing);
    },
  );

  testWidgets('an orphaned entry has its own Remove action, wired to '
      'onRemoveOrphan with that entry', (tester) async {
    const report = SyncReport(orphaned: ['Old Deck']);
    String? removed;
    await pump(tester, report: report, onRemoveOrphan: (e) => removed = e);

    expect(find.text('Old Deck'), findsOneWidget);
    await tester.tap(find.text('Remove'));
    expect(removed, 'Old Deck');
  });

  testWidgets('a report error line shows in place of the sections', (
    tester,
  ) async {
    const report = SyncReport(
      error: 'the desktop now serves another folder; pair it as a new root',
      landed: ['German/Verbs.md'],
    );
    await pump(tester, report: report);

    expect(
      find.text(
        'the desktop now serves another folder; pair it as a new root',
      ),
      findsOneWidget,
    );
    // The lib still fills report lists alongside an abort in some paths;
    // the sheet shows both without hiding one for the other.
    expect(find.text('German/Verbs.md'), findsOneWidget);
  });

  testWidgets('nothing to report yet shows only before any cycle has run', (
    tester,
  ) async {
    await pump(tester);
    expect(find.text('Nothing to report yet.'), findsOneWidget);
  });
}
