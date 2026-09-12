import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bridge/sync_bridge.dart' as sync_bridge;
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/sync/sync_controller.dart';
import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync/sync_sheet.dart';
import 'package:alix_mobile/sync_client.dart';

import 'support/deck_fixture.dart';
import 'support/fake_sync_port.dart';

class _UnusedSyncClient implements SyncClient {
  @override
  Future<SyncEntries> entries() => throw UnimplementedError();

  @override
  Future<int> pull(
    String entry,
    File target, {
    void Function(int received, int total)? onProgress,
  }) => throw UnimplementedError();

  @override
  Future<SyncPushResult> push(
    String deckId,
    List<int> document, {
    required String rootId,
    required String pulledRevision,
  }) => throw UnimplementedError();
}

void main() {
  setUpAll(() async => RustLib.init());

  late Directory scratch;

  setUp(() {
    scratch = Directory.systemTemp.createTempSync('alix-round3-review-');
  });

  tearDown(() {
    if (scratch.existsSync()) scratch.deleteSync(recursive: true);
  });

  SyncPushPlanItem item(String deckId, {required String entry, int? base}) {
    final document = File('${scratch.path}/$deckId.json')
      ..writeAsStringSync('{}');
    return SyncPushPlanItem(
      deckId: deckId,
      entry: entry,
      document: document.path,
      base: base,
      phoneRevision: 1,
    );
  }

  test('the real sync bridge resolves a loose deck title', () {
    writeTestDeck(
      '${scratch.path}/greek.md',
      '---\ntitle: Greek\n---\n## letter?\nalpha\n',
    );
    final port = sync_bridge.SyncBridgePort(
      config: const ServerConfig(
        host: '127.0.0.1',
        port: 7777,
        token: 'test',
        rootId: 'root-test',
      ),
      rootDir: scratch.path,
      client: _UnusedSyncClient(),
    );

    expect(port.deckTitle('greek.md'), 'Greek');
  });

  test(
    'resolving one of two loose decks with the same title keeps the other '
    'conflict in the report and status',
    () async {
      var firstPending = true;
      final port = FakeSyncPort(rootId: 'root-test', rootDir: scratch.path);
      port.pairedEntriesImpl = () => [
        SyncEntryState(
          entry: 'greek-a.md',
          kind: 'deck',
          digest: 'xxh64-0000000000000001',
          decks: [
            SyncDeckState(
              deckId: 'deck-a',
              path: 'greek-a.md',
              unpushed: true,
              conflict: firstPending
                  ? const PairedConflictPush(desktopRevision: 4)
                  : null,
            ),
          ],
        ),
        const SyncEntryState(
          entry: 'greek-b.md',
          kind: 'deck',
          digest: 'xxh64-0000000000000001',
          decks: [
            SyncDeckState(
              deckId: 'deck-b',
              path: 'greek-b.md',
              unpushed: true,
              conflict: PairedConflictPush(desktopRevision: 4),
            ),
          ],
        ),
      ];
      port.deckTitleImpl = (_) => 'Greek';
      port.planPushesImpl = () => [
        item('deck-a', entry: 'greek-a.md'),
        item('deck-b', entry: 'greek-b.md'),
      ];
      port.pushImpl = (deckId, _, _) async =>
          SyncPushConflict(deckId: deckId, desktopRevision: 4);
      port.pairedOrphansImpl = (_) => const [];
      port.resolveConflictImpl = (deckId, keepPhone) {
        firstPending = false;
        return const SyncResolutionDone();
      };
      final controller = SyncController(port: port);

      await controller.cycle();
      expect(controller.lastReport?.conflicts, ['Greek', 'Greek']);

      await controller.resolve('deck-a', keepPhone: true);

      expect(controller.pendingConflicts.map((c) => c.deckId), ['deck-b']);
      expect(controller.lastReport?.conflicts, ['Greek']);
      expect(controller.statusLine, 'Synced: 1 conflict');
    },
  );

  test(
    'a keep-phone push that conflicts again keeps the fresh conflict in '
    'the report and status',
    () async {
      List<SyncEntryState> pending() => const [
        SyncEntryState(
          entry: 'Biology',
          kind: 'workspace',
          digest: 'xxh64-0000000000000001',
          decks: [
            SyncDeckState(
              deckId: 'deck-a',
              path: 'decks/a.md',
              unpushed: true,
              conflict: PairedConflictPush(desktopRevision: 4),
            ),
          ],
        ),
      ];
      final port = FakeSyncPort(rootId: 'root-test', rootDir: scratch.path);
      port.pairedEntriesImpl = pending;
      port.planPushesImpl = () => [item('deck-a', entry: 'Biology')];
      port.pushImpl = (deckId, _, _) async =>
          SyncPushConflict(deckId: deckId, desktopRevision: 5);
      port.pairedOrphansImpl = (_) => const [];
      port.resolveConflictImpl = (deckId, keepPhone) =>
          SyncResolutionPush(item(deckId, entry: 'Biology', base: 4));
      final controller = SyncController(port: port);

      await controller.cycle();
      expect(controller.lastReport?.conflicts, ['Biology/a.md']);

      await controller.resolve('deck-a', keepPhone: true);

      expect(controller.pendingConflicts, hasLength(1));
      expect(controller.lastReport?.conflicts, ['Biology/a.md']);
      expect(controller.statusLine, 'Synced: 1 conflict');
    },
  );

  testWidgets(
    'the conflict writer timestamp fits on a 1080px phone at 3x density',
    (tester) async {
      final view = tester.view;
      addTearDown(view.resetPhysicalSize);
      addTearDown(view.resetDevicePixelRatio);
      view.physicalSize = const Size(1080, 2424);
      view.devicePixelRatio = 3;
      final conflict = SyncPendingConflict(
        deckId: 'deck-a',
        label: 'German/Verbs.md',
        conflict: PairedConflictPush(
          desktopWriter: SyncWriter(
            device: 'alix-workstation-1a2b3cd',
            atMs: DateTime(2026, 9, 7, 13, 19).millisecondsSinceEpoch,
          ),
        ),
      );
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: SyncReportSheet(
              report: null,
              conflicts: [conflict],
              onResolve: (_, _) async {},
              onRemoveOrphan: (_) {},
            ),
          ),
        ),
      );

      final subtitle =
          "discards the desktop's, last written by "
          'alix-workstation-1a2b3cd at 2026-09-07 13:19';
      final finder = find.text(subtitle);
      final text = tester.widget<Text>(finder);
      final paragraph = tester.renderObject<RenderParagraph>(finder);
      final painter = TextPainter(
        text: TextSpan(text: text.data, style: text.style),
        maxLines: text.maxLines,
        textDirection: TextDirection.ltr,
      )..layout(maxWidth: paragraph.size.width);

      expect(painter.didExceedMaxLines, isFalse);
    },
  );
}
