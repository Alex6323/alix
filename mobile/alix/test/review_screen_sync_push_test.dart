// The review summary's silent post-session push (lib/review_screen.dart's
// `_maybePushSummary`, wired to a `SyncController` passed in from a paired
// root): fires exactly once even though it listens for the whole session's
// notifyListeners stream, resolves the reviewed deck's id from its path,
// and opens the same conflict choice sync_sheet.dart uses when the push
// comes back 409. Driven with a FakeSyncPort, no network and no dylib-backed
// sync port; RustLib.init() is still required for ReviewScreen's own
// listing/session calls, same as the other review_screen_*_test.dart files.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/sync/sync_controller.dart';
import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync/sync_sheet.dart';
import 'package:alix_mobile/sync_client.dart';
import 'package:alix_mobile/theme.dart';

import 'support/deck_fixture.dart';
import 'support/fake_sync_port.dart';

void main() {
  setUpAll(() async => RustLib.init());

  Directory tempDir(String prefix) {
    final dir = Directory.systemTemp.createTempSync(prefix);
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  /// A one-card deck whose progress document, once pushed, resolves back to
  /// `deckId` `'deck-1'` via `deckIdForPath` (a loose deck flattens directly
  /// under `rootDir`, matching `PairedRoot::entry_root`).
  (Directory root, FakeSyncPort port) pairedDeck(Directory tmp) {
    final root = Directory('${tmp.path}/root')..createSync();
    writeTestDeck('${root.path}/facts.md', '## q?\na\n');
    final progressDoc = File('${tmp.path}/progress.json')
      ..writeAsStringSync('{}');
    final port = FakeSyncPort(rootId: 'root-test0000000000000000000000', rootDir: root.path);
    port.pairedEntriesImpl = () => [
      SyncEntryState(
        entry: 'facts',
        kind: 'deck',
        digest: 'xxh64-0000000000000001',
        decks: [
          SyncDeckState(deckId: 'deck-1', path: 'facts.md', unpushed: true),
        ],
      ),
    ];
    port.planPushesImpl = () => [
      SyncPushPlanItem(
        deckId: 'deck-1',
        entry: 'facts',
        document: progressDoc.path,
        base: null,
        phoneRevision: 1,
      ),
    ];
    return (root, port);
  }

  // The summary push reads the progress document off disk with a real
  // `File.readAsBytes`, fired and forgotten from the grade that lands on
  // the summary. `pumpAndSettle` alone only waits out scheduled frames,
  // not an unrelated pending Future, and the fake test zone never
  // services genuine dart:io I/O on its own -- so a case that expects the
  // push to have happened polls [settled] inside `runAsync`, bounded so a
  // real stall fails the test instead of hanging it.
  Future<void> finishReview(WidgetTester tester, {bool Function()? settled}) async {
    await tester.tap(find.text('Reveal'));
    await tester.pumpAndSettle();
    if (settled == null) {
      await tester.tap(find.text('Seen'));
      await tester.pumpAndSettle();
      return;
    }
    await tester.runAsync(() async {
      await tester.tap(find.text('Seen'));
      final deadline = DateTime.now().add(const Duration(seconds: 2));
      while (!settled() && DateTime.now().isBefore(deadline)) {
        await Future<void>.delayed(const Duration(milliseconds: 5));
      }
      await tester.pumpAndSettle();
    });
  }

  testWidgets(
    'the summary push fires exactly once, even though the listener runs on '
    'every notify across the whole session',
    (tester) async {
      final tmp = tempDir('alix-sync-push-');
      final (root, port) = pairedDeck(tmp);
      final syncController = SyncController(port: port);
      addTearDown(syncController.dispose);

      await tester.pumpWidget(
        MaterialApp(
          theme: alixDark(),
          home: ReviewScreen(
            deckPath: '${root.path}/facts.md',
            rootDir: root.path,
            depth: ReviewDepth.recall,
            supportDir: tempDir('alix-sync-push-support-'),
            syncController: syncController,
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(port.pushCalls, isEmpty, reason: 'not on the summary yet');

      await finishReview(tester, settled: () => port.pushCalls.isNotEmpty);
      expect(find.text('SESSION COMPLETE'), findsOneWidget);
      expect(port.pushCalls, ['deck-1']);

      // Further idle settling on the summary must not push again.
      await tester.pump(const Duration(seconds: 1));
      await tester.pumpAndSettle();
      expect(port.pushCalls, ['deck-1']);
    },
  );

  testWidgets(
    'a deck outside the paired entries is never pushed (deckIdForPath finds '
    'nothing to push)',
    (tester) async {
      final tmp = tempDir('alix-sync-push-unpaired-');
      final root = Directory('${tmp.path}/root')..createSync();
      writeTestDeck('${root.path}/other.md', '## q?\na\n');
      final (_, port) = pairedDeck(tmp);
      final syncController = SyncController(port: port);
      addTearDown(syncController.dispose);

      await tester.pumpWidget(
        MaterialApp(
          theme: alixDark(),
          home: ReviewScreen(
            deckPath: '${root.path}/other.md',
            rootDir: root.path,
            depth: ReviewDepth.recall,
            supportDir: tempDir('alix-sync-push-unpaired-support-'),
            syncController: syncController,
          ),
        ),
      );
      await tester.pumpAndSettle();
      await finishReview(tester);

      expect(port.pushCalls, isEmpty);
    },
  );

  testWidgets(
    'a 409 on the summary push opens the same conflict choice the sync '
    'report sheet uses, naming what each side discards',
    (tester) async {
      final tmp = tempDir('alix-sync-push-conflict-');
      final (root, port) = pairedDeck(tmp);
      port.pushImpl = (deckId, document, pulledRevision) async =>
          SyncPushConflict(deckId: deckId, desktopRevision: 3);
      // pendingConflicts reads live from pairedEntries(), so the fake must
      // already carry the conflict a real push would only learn about from
      // the desktop's response.
      port.pairedEntriesImpl = () => [
        SyncEntryState(
          entry: 'facts',
          kind: 'deck',
          digest: 'xxh64-0000000000000001',
          decks: [
            SyncDeckState(
              deckId: 'deck-1',
              path: 'facts.md',
              unpushed: true,
              conflict: const PairedConflictPush(desktopRevision: 3),
            ),
          ],
        ),
      ];

      final syncController = SyncController(port: port);
      addTearDown(syncController.dispose);

      await tester.pumpWidget(
        MaterialApp(
          theme: alixDark(),
          home: ReviewScreen(
            deckPath: '${root.path}/facts.md',
            rootDir: root.path,
            depth: ReviewDepth.recall,
            supportDir: tempDir('alix-sync-push-conflict-support-'),
            syncController: syncController,
          ),
        ),
      );
      await tester.pumpAndSettle();
      await finishReview(tester, settled: () => port.pushCalls.isNotEmpty);

      expect(find.byType(SyncReportSheet), findsOneWidget);
      expect(find.text("Keep the phone's progress"), findsOneWidget);
      expect(find.text("discards the desktop's version"), findsOneWidget);
    },
  );
}
