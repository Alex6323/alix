// Tier-3 mobile e2e for phone sync (docs/API.md section 4.12, book chapter
// 18): the REAL desktop binary from this checkout serves a temp folder over
// its real HTTP API while the REAL app runs in a device window and drives
// the whole loop through its own widgets -- list, pull, review, push, and an
// explicit conflict choice. Nothing here is faked: no FakeSyncPort, no fake
// server client, no hand-written progress documents. The one injection is
// `AlixApp.supportDir`, the temp-support-dir seam `persistTheme` and
// `PickerScreen.supportDir` already established, so the run never touches
// the developer's own pairings.
//
// Timeline of the served deck's progress document, since every assertion
// below is about which side wrote it last:
//
//   seed (introduced 20 min "ago", so a quiz is due at once, no sleeping)
//     -> phone pulls it, reviews at Recall, pushes from the summary
//     -> desktop reviews at Reconstruct (the depth the Recall pass leaves
//        unscheduled) through /api/select + /api/grade, so both sides moved
//     -> phone reviews at Reconstruct too: its push is refused 409
//     -> keep-phone, and the phone's document wins on the desktop.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/bridge/sync_bridge.dart' as sync_bridge;
import 'package:alix_mobile/main.dart';
import 'package:alix_mobile/picker/picker_widgets.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/sync/sync_models.dart'
    show conflictTakeDesktopWording;

import '../test/support/deck_fixture.dart';
import 'sync_e2e_support.dart';

const _phoneDevice = 'e2e-phone';
const _seedDevice = 'e2e-desktop-seed';
const _token = '0123456789abcdef0123456789abcdef';
const _keepPhonePrefix = "Keep the phone's progress";

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets(
    'a paired phone pulls a desktop entry, pushes its review back, and '
    'resolves a two-sided conflict by keeping the phone',
    (tester) async {
      await RustLib.init();

      // ---------------------------------------------------------------
      // Step 1: the desktop. A temp served folder with one loose deck and
      // one workspace, both initialized by the real binary so their deck
      // and card ids are minted by the lib, never hand-written.
      // ---------------------------------------------------------------
      final scratch = Directory.systemTemp.createTempSync('alix-sync-e2e-');
      addTearDown(() {
        if (scratch.existsSync()) scratch.deleteSync(recursive: true);
      });
      final desktop = Directory('${scratch.path}/desktop')
        ..createSync(recursive: true);
      final phoneRoot = Directory('${scratch.path}/phone-decks')
        ..createSync(recursive: true);
      final support = Directory('${scratch.path}/support')
        ..createSync(recursive: true);

      final looseDeck = '${desktop.path}/greek.md';
      File(looseDeck).writeAsStringSync(
        '---\ntitle: Greek\n---\n\n## capital of greece?\nAthens\n',
      );
      runAlix(['workspace', 'init', '${desktop.path}/Biology', '--title', 'Biology']);
      File('${desktop.path}/Biology/decks/cells.md').writeAsStringSync(
        '---\ntitle: Cells\n---\n\n## powerhouse of the cell?\nMitochondrion\n',
      );
      runAlix(['deck', 'init', looseDeck]);
      runAlix(['deck', 'init', '${desktop.path}/Biology/decks/cells.md']);
      final deckId = deckIdOf(looseDeck);

      // The desktop's own prior progress: the card is introduced twenty
      // minutes "ago" through the core, so it is due the moment the phone
      // opens it. Generated state, never a committed fixture document.
      final backdated =
          BigInt.from(DateTime.now().millisecondsSinceEpoch - 20 * 60 * 1000);
      ReviewSession.open(
        deckPath: looseDeck,
        rootDir: desktop.path,
        nowMs: backdated,
        device: _seedDevice,
      ).introduce(nowMs: backdated);
      final seeded = progressDocument(desktop.path, deckId);
      expect(
        seeded,
        isNotNull,
        reason: 'step 1: seeding wrote no $deckId.json under ${desktop.path}',
      );
      final seededRevision = seeded!['revision'] as int;

      final server = await DesktopServer.start(
        folder: desktop.path,
        configPath: '${scratch.path}/config.toml',
        token: _token,
      );
      addTearDown(server.stop);
      final pairedDir = sync_bridge.pairedRootDirFor(
        support: support.path,
        rootId: server.rootId,
      );

      // ---------------------------------------------------------------
      // Step 2: the phone. One deck of its own, plus a pairing for the
      // running server stored exactly as the pairing sheet stores it.
      // ---------------------------------------------------------------
      writeTestDeck(
        '${phoneRoot.path}/notes.md',
        '---\ntitle: Phone Notes\n---\n## a phone-only card?\nyes\n',
      );
      await savePairing(
        ServerConfig(
          host: '127.0.0.1',
          port: server.port,
          token: _token,
          rootId: server.rootId,
        ),
        support: support,
      );

      await tester.pumpWidget(
        AlixApp(
          prepared: Prepared(root: phoneRoot.path, device: _phoneDevice),
          supportDir: support,
        ),
      );
      await tester.pumpAndSettle();

      // ---------------------------------------------------------------
      // Step 3a: the picker lists the phone's own entries first, then the
      // paired desktop's two entries as never-pulled rows.
      // ---------------------------------------------------------------
      expect(
        find.text('Phone Notes'),
        findsOneWidget,
        reason: "step 3a: the phone's own root is not listed",
      );
      await waitUntil(
        tester,
        "the app-open cycle to list the desktop's entries",
        () => tester.any(find.byType(PickerAvailableEntryRow)),
      );
      expect(
        find.text('127.0.0.1:${server.port}'),
        findsOneWidget,
        reason: 'step 3a: the paired section carries no host:port heading',
      );
      expect(
        find.byType(PickerAvailableEntryRow),
        findsNWidgets(2),
        reason:
            'step 3a: the desktop serves greek.md and Biology, so both are '
            'never-pulled rows before any pull',
      );
      expect(
        find.text('greek.md'),
        findsOneWidget,
        reason: 'step 3a: the loose deck is missing from the paired section',
      );
      expect(
        find.text('Biology'),
        findsOneWidget,
        reason: 'step 3a: the workspace is missing from the paired section',
      );
      expect(
        tester.getTopLeft(find.text('Phone Notes')).dy,
        lessThan(tester.getTopLeft(find.text('greek.md')).dy),
        reason:
            "step 3a: the phone's own entries must sit above the paired "
            "desktop's, never merged into one list",
      );

      // ---------------------------------------------------------------
      // Step 3b: tapping a never-pulled row pulls that entry.
      // ---------------------------------------------------------------
      await tester.tap(find.text('greek.md'));
      await tester.pumpAndSettle();
      final manifest = File('$pairedDir/.alix/pull/greek.md.json');
      await waitUntil(
        tester,
        'the pull of greek.md to land under $pairedDir',
        () => manifest.existsSync() && File('$pairedDir/greek.md').existsSync(),
      );
      expect(
        progressDocument(pairedDir, deckId),
        isNotNull,
        reason:
            "step 3b: the pull must carry the desktop's progress document, "
            'not only the deck file',
      );
      final firstManifest =
          jsonDecode(manifest.readAsStringSync()) as Map<String, dynamic>;
      expect(
        firstManifest['root_id'],
        server.rootId,
        reason: "step 3b: the manifest names another root than the pairing's",
      );

      await waitUntil(
        tester,
        'the pulled entry to become an ordinary picker row',
        () => tester.any(find.text('Greek')),
      );
      expect(
        find.byType(PickerAvailableEntryRow),
        findsNWidgets(1),
        reason:
            'step 3b: greek.md is pulled now, so only Biology stays a '
            'never-pulled row',
      );

      // ---------------------------------------------------------------
      // Step 3c: open the pulled deck, grade one card, reach the summary.
      // The push fires from the review screen itself; nothing here calls it.
      // ---------------------------------------------------------------
      await tester.tap(find.text('Greek'));
      await tester.pumpAndSettle();
      expect(
        find.text('capital of greece?'),
        findsOneWidget,
        reason:
            'step 3c: the pulled deck served no due card; the seeded '
            'introduction should have made one due',
      );
      await tester.tap(find.text('Reveal'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Got it'));
      await tester.pumpAndSettle();
      expect(
        find.text('SESSION COMPLETE'),
        findsOneWidget,
        reason: 'step 3c: the session did not reach its summary',
      );

      await waitUntil(
        tester,
        "the summary push to reach the desktop's progress document",
        () =>
            progressDocument(desktop.path, deckId)?['writer']['device'] ==
            _phoneDevice,
      );
      final pushed = progressDocument(desktop.path, deckId)!;
      expect(
        pushed['revision'],
        greaterThan(seededRevision),
        reason:
            'step 3c: an accepted push writes pulled revision + 1; the '
            'desktop still reads revision ${pushed['revision']} against the '
            'seeded $seededRevision',
      );

      // ---------------------------------------------------------------
      // Step 4a: the desktop moves. A real browser-shaped review of the
      // same deck at the depth the phone's Recall pass left unscheduled.
      // ---------------------------------------------------------------
      await tester.tap(find.byType(BackButton));
      await tester.pumpAndSettle();
      await server.reviewOnDesktop('greek.md');
      final movedByDesktop = progressDocument(desktop.path, deckId)!;
      expect(
        movedByDesktop['revision'],
        greaterThan(pushed['revision'] as int),
        reason: 'step 4a: the desktop review did not advance the revision',
      );
      expect(
        movedByDesktop['writer']['device'],
        isNot(_phoneDevice),
        reason: 'step 4a: the desktop review left the phone as last writer',
      );

      // ---------------------------------------------------------------
      // Step 4b: the phone moves too, at Reconstruct depth. Its summary
      // push is refused (409) and the choice sheet names both sides.
      // ---------------------------------------------------------------
      await tester.longPress(find.text('Greek'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Reconstruct'));
      await tester.pumpAndSettle();
      expect(
        find.text('capital of greece?'),
        findsOneWidget,
        reason: 'step 4b: the deck served no card at Reconstruct depth',
      );
      final attempt = find.byType(TextField);
      expect(
        attempt,
        findsOneWidget,
        reason: 'step 4b: Reconstruct on a one-line answer is a typed check',
      );
      await tester.enterText(attempt, 'Athens');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Submit'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Got it'));
      await tester.pumpAndSettle();
      expect(
        find.text('SESSION COMPLETE'),
        findsOneWidget,
        reason: 'step 4b: the second session did not reach its summary',
      );

      await waitUntil(
        tester,
        'the refused summary push to raise the conflict choice',
        () => tester.any(find.textContaining(_keepPhonePrefix)),
      );
      expect(
        find.text(conflictTakeDesktopWording.title),
        findsOneWidget,
        reason:
            'step 4b: the choice must name what taking the desktop discards',
      );
      final conflicts = File('$pairedDir/.alix/conflicts.json');
      expect(
        conflicts.existsSync() && conflicts.readAsStringSync().contains(deckId),
        isTrue,
        reason: 'step 4b: the refusal recorded no conflict mark for $deckId',
      );

      // Leave the choice unmade here: the same conflict must survive to the
      // Sync run below, which is the path a learner reaches it by later.
      await tester.tapAt(const Offset(20, 20));
      await tester.pumpAndSettle();
      await tester.tap(find.byType(BackButton));
      await tester.pumpAndSettle();

      // ---------------------------------------------------------------
      // Step 4c: Sync from the row's own menu, then resolve keep-phone
      // from the report sheet.
      // ---------------------------------------------------------------
      final desktopRevisionBefore = movedByDesktop['revision'] as int;
      await tester.tap(find.byIcon(Icons.more_vert));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Sync'));
      await tester.pumpAndSettle();
      await waitUntil(
        tester,
        "the Sync cycle to re-read the desktop's revision for greek.md",
        () {
          final fresh =
              jsonDecode(manifest.readAsStringSync()) as Map<String, dynamic>;
          final decks = fresh['decks'] as List<dynamic>;
          final row = decks.first as Map<String, dynamic>;
          return row['revision'] == desktopRevisionBefore;
        },
      );
      expect(
        progressDocument(pairedDir, deckId)?['writer']['device'],
        _phoneDevice,
        reason:
            "step 4c: the pull must not overwrite the phone's own unpushed "
            'document while the deck is conflicted',
      );

      await tester.tap(find.byKey(const Key('sync-status')));
      await tester.pumpAndSettle();
      expect(
        find.textContaining(_keepPhonePrefix),
        findsOneWidget,
        reason:
            'step 4c: the Sync report must still offer the conflict choice',
      );
      await tester.tap(find.textContaining(_keepPhonePrefix));
      await tester.pumpAndSettle();

      await waitUntil(
        tester,
        "keep-phone to write the phone's document onto the desktop",
        () {
          final doc = progressDocument(desktop.path, deckId);
          return doc != null &&
              doc['writer']['device'] == _phoneDevice &&
              (doc['revision'] as int) > desktopRevisionBefore;
        },
      );
      await waitUntil(
        tester,
        "the phone's conflict mark for $deckId to clear",
        () =>
            !conflicts.existsSync() ||
            !conflicts.readAsStringSync().contains(deckId),
      );
    },
    timeout: const Timeout(Duration(minutes: 4)),
  );
}
