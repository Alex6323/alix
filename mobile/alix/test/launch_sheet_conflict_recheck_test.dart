import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/sync/sync_controller.dart';
import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync/sync_sheet.dart';
import 'package:alix_mobile/sync_client.dart';
import 'package:alix_mobile/walk_screen.dart';

import 'support/deck_fixture.dart';
import 'support/fake_sync_port.dart';
import 'support/picker_listing.dart';

void main() {
  setUpAll(() async => RustLib.init());

  for (final (row, session) in [
    ('Recall', ReviewScreen),
    ('Walk', WalkScreen),
  ]) {
    testWidgets(
      'a paired deck that conflicts while its launch sheet is open never '
      'starts a session from the $row row',
      (tester) async {
        final root = Directory.systemTemp.createTempSync(
          'alix-launch-conflict-root-',
        );
        final support = Directory.systemTemp.createTempSync(
          'alix-launch-conflict-support-',
        );
        addTearDown(() {
          if (root.existsSync()) root.deleteSync(recursive: true);
          if (support.existsSync()) support.deleteSync(recursive: true);
        });
        final workspace = Directory('${root.path}/ws');
        Directory('${workspace.path}/decks').createSync(recursive: true);
        File('${workspace.path}/alix.toml').writeAsStringSync('title = "Ws"\n');
        writeTestDeck(
          '${workspace.path}/decks/deck.md',
          '---\ntitle: Deck\n---\n## q\na\n',
        );

        var conflicted = false;
        final port = FakeSyncPort(rootDir: root.path);
        port.entriesImpl = () async => const SyncEntries(
          rootId: 'root-test0000000000000000000000',
          entries: [
            SyncEntry(
              name: 'ws',
              kind: 'workspace',
              members: 1,
              unpackedBytes: 10,
              digest: 'xxh64-0000000000000001',
              leftOut: [],
            ),
          ],
        );
        port.pairedEntriesImpl = () => [
          SyncEntryState(
            entry: 'ws',
            kind: 'workspace',
            digest: 'xxh64-0000000000000001',
            decks: [
              SyncDeckState(
                deckId: 'deck-1',
                path: 'decks/deck.md',
                unpushed: false,
                conflict: conflicted
                    ? const PairedConflictPush(desktopRevision: 2)
                    : null,
              ),
            ],
          ),
        ];
        final syncController = SyncController(port: port);
        addTearDown(syncController.dispose);

        await tester.pumpWidget(
          MaterialApp(
            home: PickerScreen(
              root: root.path,
              dir: workspace.path,
              title: 'Ws',
              supportDir: support,
              syncController: syncController,
              isPairedSubtree: true,
              isPushedRoute: true,
            ),
          ),
        );
        await settlePicker(tester);

        await tester.tap(find.text('Deck'));
        await tester.pumpAndSettle();
        expect(find.text('Recall'), findsOneWidget);

        conflicted = true;
        await syncController.cycle();
        expect(syncController.pendingConflicts, hasLength(1));

        await tester.tap(find.text(row));
        await tester.pumpAndSettle();

        expect(find.byType(session), findsNothing, reason: '$row row');
        expect(
          find.byType(SyncReportSheet),
          findsOneWidget,
          reason: '$row row',
        );
      },
    );
  }
}
