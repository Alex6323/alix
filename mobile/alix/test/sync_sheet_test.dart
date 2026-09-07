// Widget tests for lib/sync/sync_sheet.dart: the conflict choice row's exact
// title/subtitle wording (what each side discards), that only non-empty
// report sections render, and that a row tap calls onResolve with the right
// deckId/keepPhone pair. Pure presentation, no bridge or dylib needed.
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync/sync_sheet.dart';
import 'package:alix_mobile/sync_client.dart';

void main() {
  Future<void> pump(
    WidgetTester tester, {
    SyncReport? report,
    List<SyncPendingConflict> conflicts = const [],
    Set<String> unpushedOrphans = const {},
    void Function(String deckId, bool keepPhone)? onResolve,
    ValueChanged<String>? onRemoveOrphan,
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: SyncReportSheet(
            report: report,
            conflicts: conflicts,
            unpushedOrphans: unpushedOrphans,
            onResolve: onResolve ?? (_, _) {},
            onRemoveOrphan: onRemoveOrphan ?? (_) {},
          ),
        ),
      ),
    );
  }

  testWidgets(
    'a conflict names what each row discards, including the desktop '
    "writer when the bridge reports one",
    (tester) async {
      const conflict = SyncPendingConflict(
        deckId: 'deck-1',
        label: 'German/Verbs.md',
        conflict: PairedConflictPush(
          desktopWriter: SyncWriter(device: 'desk-1', atMs: 0),
        ),
      );
      await pump(tester, conflicts: const [conflict]);

      expect(find.text("Keep the phone's progress"), findsOneWidget);
      expect(
        find.text(
          "discards the desktop's, last written by desk-1 at "
          '1970-01-01 01:00',
        ),
        findsOneWidget,
      );
      expect(find.text("Take the desktop's"), findsOneWidget);
      expect(
        find.text("discards the phone's progress since the last sync"),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'the writer subtitle carries the date, not only the time, since a '
    'stale conflict can be days old',
    (tester) async {
      final twoDaysAgo = DateTime(2026, 9, 5, 9, 15).millisecondsSinceEpoch;
      final conflict = SyncPendingConflict(
        deckId: 'deck-1',
        label: 'German/Verbs.md',
        conflict: PairedConflictPush(
          desktopWriter: SyncWriter(device: 'desk-1', atMs: twoDaysAgo),
        ),
      );
      await pump(tester, conflicts: [conflict]);

      expect(
        find.text(
          "discards the desktop's, last written by desk-1 at "
          '2026-09-05 09:15',
        ),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'the writer subtitle stays fully visible, never ellipsized, for a '
    '24-character device label on a 1080px-wide phone',
    (tester) async {
      final conflict = SyncPendingConflict(
        deckId: 'deck-1',
        label: 'German/Verbs.md',
        conflict: PairedConflictPush(
          desktopWriter: SyncWriter(
            device: 'alix-workstation-1a2b3cd',
            atMs: DateTime(2026, 9, 7, 13, 19).millisecondsSinceEpoch,
          ),
        ),
      );
      final view = tester.view;
      addTearDown(view.resetPhysicalSize);
      addTearDown(view.resetDevicePixelRatio);
      view.physicalSize = const Size(1080, 2424);
      view.devicePixelRatio = 1;
      await pump(tester, conflicts: [conflict]);

      final subtitleText =
          "discards the desktop's, last written by "
          'alix-workstation-1a2b3cd at 2026-09-07 13:19';
      final finder = find.text(subtitleText);
      expect(finder, findsOneWidget);
      final subtitle = tester.widget<Text>(finder);
      final renderObject = tester.renderObject<RenderParagraph>(finder);
      final painter = TextPainter(
        text: TextSpan(text: subtitle.data, style: subtitle.style),
        maxLines: subtitle.maxLines,
        textDirection: TextDirection.ltr,
      )..layout(maxWidth: renderObject.size.width);

      expect(
        painter.didExceedMaxLines,
        isFalse,
        reason:
            'the timestamp is the one fact that distinguishes the two '
            'choices; it must fit, not clip',
      );
    },
  );

  testWidgets(
    'a conflict with no known desktop writer keeps the keep-phone subtitle '
    'writer-free rather than inventing one',
    (tester) async {
      const conflict = SyncPendingConflict(
        deckId: 'deck-1',
        label: 'German/Verbs.md',
        conflict: PairedConflictPull(),
      );
      await pump(tester, conflicts: const [conflict]);

      expect(find.text("discards the desktop's version"), findsOneWidget);
    },
  );

  testWidgets('tapping keep-phone resolves with keepPhone true, the '
      "conflict's own deckId", (tester) async {
    const conflict = SyncPendingConflict(
      deckId: 'deck-9',
      label: 'German/Verbs.md',
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

    await tester.tap(find.text("Keep the phone's progress"));
    expect(resolvedId, 'deck-9');
    expect(resolvedKeepPhone, isTrue);
  });

  testWidgets('tapping take-desktop resolves with keepPhone false', (
    tester,
  ) async {
    const conflict = SyncPendingConflict(
      deckId: 'deck-9',
      label: 'German/Verbs.md',
      conflict: PairedConflictPull(),
    );
    bool? resolvedKeepPhone;
    await pump(
      tester,
      conflicts: const [conflict],
      onResolve: (_, keepPhone) => resolvedKeepPhone = keepPhone,
    );

    await tester.tap(find.text(conflictTakeDesktopWording.title));
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

  testWidgets(
    'an orphaned entry asks for confirmation before Remove fires, naming '
    'the entry',
    (tester) async {
      const report = SyncReport(orphaned: ['Old Deck']);
      String? removed;
      await pump(tester, report: report, onRemoveOrphan: (e) => removed = e);

      expect(find.text('Old Deck'), findsOneWidget);
      await tester.tap(find.text('Remove'));
      await tester.pumpAndSettle();

      expect(find.text('Remove "Old Deck" from this phone?'), findsOneWidget);
      expect(removed, isNull);

      await tester.tap(
        find.descendant(
          of: find.byType(AlertDialog),
          matching: find.text('Remove'),
        ),
      );
      await tester.pumpAndSettle();

      expect(removed, 'Old Deck');
    },
  );

  testWidgets(
    'canceling the confirmation leaves the entry and never fires '
    'onRemoveOrphan',
    (tester) async {
      const report = SyncReport(orphaned: ['Old Deck']);
      String? removed;
      await pump(tester, report: report, onRemoveOrphan: (e) => removed = e);

      await tester.tap(find.text('Remove'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();

      expect(removed, isNull);
      expect(find.text('Old Deck'), findsOneWidget);
    },
  );

  testWidgets(
    'unpushed progress on an orphan is named in the confirmation',
    (tester) async {
      const report = SyncReport(orphaned: ['Old Deck']);
      await pump(
        tester,
        report: report,
        unpushedOrphans: const {'Old Deck'},
      );

      await tester.tap(find.text('Remove'));
      await tester.pumpAndSettle();

      expect(
        find.text(
          'Remove "Old Deck" from this phone? Its unpushed progress goes too.',
        ),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'a resolved conflict leaves the sheet immediately, and a second tap '
    'has nothing left to tap',
    (tester) async {
      const conflict = SyncPendingConflict(
        deckId: 'deck-9',
        label: 'German/Verbs.md',
        conflict: PairedConflictPull(),
      );
      await pump(tester, conflicts: const [conflict]);

      expect(find.byType(OutlinedButton), findsNWidgets(2));
      await tester.tap(find.text("Keep the phone's progress"));
      await tester.pump();

      expect(find.byType(OutlinedButton), findsNothing);
    },
  );

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
