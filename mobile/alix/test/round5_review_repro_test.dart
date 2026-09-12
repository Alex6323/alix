import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/bridge/sync_bridge.dart' as sync_bridge;
import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/sync/sync_controller.dart';
import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync_client.dart';

import 'support/fake_server_client.dart';
import 'support/fake_sync_port.dart';
import 'support/picker_listing.dart';

void main() {
  setUp(answerPathProvider);
  setUpAll(() async => RustLib.init());

  Directory tempDir(String prefix) {
    final dir = Directory.systemTemp.createTempSync(prefix);
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  File document(Directory scratch, String name, [String content = '{}']) {
    final file = File('${scratch.path}/$name');
    file.writeAsStringSync(content);
    return file;
  }

  SyncPushPlanItem item(Directory scratch, String deckId) {
    return SyncPushPlanItem(
      deckId: deckId,
      entry: 'Biology',
      document: document(scratch, '$deckId.json').path,
      base: 1,
      phoneRevision: 1,
    );
  }

  const config = ServerConfig(
    host: '127.0.0.1',
    port: 7777,
    token: 'abc',
    rootId: 'root-test0000000000000000000000',
  );

  Future<Directory> pairedRoot(Directory support) async {
    await savePairing(config, support: support);
    return Directory(
      sync_bridge.pairedRootDirFor(
        support: support.path,
        rootId: config.rootId,
      ),
    )..createSync(recursive: true);
  }

  test('a wrong-root report names the root that answered', () async {
    final scratch = tempDir('alix-round5-root-message-');
    final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
    port.entriesImpl = () async =>
        const SyncEntries(rootId: 'root-b', entries: []);
    final controller = SyncController(port: port);

    await controller.cycle();

    expect(
      controller.lastReport?.error,
      contains('root-b'),
      reason: 'the spec requires the report to name the root that answered',
    );
  });

  test('a review saved after the cycle pushed that deck is still pushed after '
      'the cycle', () async {
    final scratch = tempDir('alix-round5-post-push-save-');
    final pullStarted = Completer<void>();
    final finishPull = Completer<void>();
    var phoneRevision = 1;
    final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
    port.entriesImpl = () async => const SyncEntries(
      rootId: 'root-a',
      entries: [
        SyncEntry(
          name: 'Biology',
          kind: 'workspace',
          members: 1,
          unpackedBytes: 10,
          digest: 'xxh64-0000000000000002',
          leftOut: [],
        ),
      ],
    );
    port.pairedEntriesImpl = () => const [
      SyncEntryState(
        entry: 'Biology',
        kind: 'workspace',
        digest: 'xxh64-0000000000000001',
        decks: [],
      ),
    ];
    port.planPushesImpl = () => [
      SyncPushPlanItem(
        deckId: 'deck-a',
        entry: 'Biology',
        document: document(scratch, 'deck-a.json', '$phoneRevision').path,
        base: 1,
        phoneRevision: phoneRevision,
      ),
    ];
    port.pullImpl = (entry, target, unpackedBytes) async {
      pullStarted.complete();
      await finishPull.future;
      await target.writeAsBytes(const []);
      return 0;
    };
    final controller = SyncController(port: port);

    final cycle = controller.cycle();
    await pullStarted.future;
    phoneRevision = 2;
    final push = controller.pushOne('deck-a');
    finishPull.complete();
    await push;
    await cycle;

    expect(
      port.pushCalls,
      ['deck-a', 'deck-a'],
      reason:
          'the second save happened after the cycle accepted revision 1, '
          'so skipping the queued trigger silently loses revision 2',
    );
  });

  test(
    'a deferred summary push completes only after its conflict is visible',
    () async {
      final scratch = tempDir('alix-round5-deferred-conflict-');
      final pullStarted = Completer<void>();
      final finishPull = Completer<void>();
      var changedAfterPlanning = false;
      var pushConflicted = false;
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-a',
        entries: [
          SyncEntry(
            name: 'Biology',
            kind: 'workspace',
            members: 1,
            unpackedBytes: 10,
            digest: 'xxh64-0000000000000002',
            leftOut: [],
          ),
        ],
      );
      port.pairedEntriesImpl = () => pushConflicted
          ? const [
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
            ]
          : const [
              SyncEntryState(
                entry: 'Biology',
                kind: 'workspace',
                digest: 'xxh64-0000000000000001',
                decks: [],
              ),
            ];
      port.planPushesImpl = () =>
          changedAfterPlanning ? [item(scratch, 'deck-a')] : const [];
      port.pushImpl = (deckId, _, _) async {
        pushConflicted = true;
        return SyncPushConflict(deckId: deckId, desktopRevision: 4);
      };
      port.pullImpl = (entry, target, unpackedBytes) async {
        pullStarted.complete();
        await finishPull.future;
        await target.writeAsBytes(const []);
        return 0;
      };
      final controller = SyncController(port: port);

      final cycle = controller.cycle();
      await pullStarted.future;
      changedAfterPlanning = true;
      bool? conflictVisibleAtCompletion;
      final summaryPush = controller.pushOne('deck-a').then((_) {
        conflictVisibleAtCompletion = controller.pendingConflicts.any(
          (conflict) => conflict.deckId == 'deck-a',
        );
      });
      await Future<void>.delayed(Duration.zero);
      final completedBeforeDrain = conflictVisibleAtCompletion;
      finishPull.complete();
      await cycle;
      await summaryPush;

      expect(
        completedBeforeDrain,
        isNull,
        reason:
            'ReviewScreen awaits this future before looking for the 409; '
            'completing while the push is merely queued hides the choice',
      );
      expect(conflictVisibleAtCompletion, isTrue);
    },
  );

  test('a new cycle cannot overlap a deferred push drain', () async {
    final scratch = tempDir('alix-round5-overlap-');
    final pullStarted = Completer<void>();
    final finishPull = Completer<void>();
    final firstPushStarted = Completer<void>();
    final finishFirstPush = Completer<void>();
    var changedAfterPlanning = false;
    var pushNumber = 0;
    final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
    port.entriesImpl = () async => const SyncEntries(
      rootId: 'root-a',
      entries: [
        SyncEntry(
          name: 'Biology',
          kind: 'workspace',
          members: 1,
          unpackedBytes: 10,
          digest: 'xxh64-0000000000000002',
          leftOut: [],
        ),
      ],
    );
    port.pairedEntriesImpl = () => const [
      SyncEntryState(
        entry: 'Biology',
        kind: 'workspace',
        digest: 'xxh64-0000000000000001',
        decks: [],
      ),
    ];
    port.planPushesImpl = () =>
        changedAfterPlanning ? [item(scratch, 'deck-a')] : const [];
    port.pushImpl = (deckId, _, _) async {
      pushNumber++;
      if (pushNumber == 1) {
        firstPushStarted.complete();
        await finishFirstPush.future;
      }
      return SyncPushAccepted(deckId: deckId, revision: pushNumber + 1);
    };
    port.pullImpl = (entry, target, unpackedBytes) async {
      pullStarted.complete();
      await finishPull.future;
      await target.writeAsBytes(const []);
      return 0;
    };
    final controller = SyncController(port: port);

    final cycle = controller.cycle();
    await pullStarted.future;
    changedAfterPlanning = true;
    final push = controller.pushOne('deck-a');
    finishPull.complete();
    await firstPushStarted.future;
    final overlappingCycle = controller.cycle();
    final entriesCallsDuringDrain = port.entriesCalls.length;
    finishFirstPush.complete();
    await push;
    await cycle;
    await overlappingCycle;

    expect(
      entriesCallsDuringDrain,
      1,
      reason:
          'the drain clears running before its request starts, so a second '
          'cycle enters while the deferred push is still in flight',
    );
  });

  testWidgets(
    're-pairing the active root at a new endpoint with the same token rebuilds '
    'its sync port',
    (tester) async {
      final support = tempDir('alix-round5-repair-endpoint-support-');
      final phoneRoot = tempDir('alix-round5-repair-endpoint-phone-');
      const oldConfig = ServerConfig(
        host: '127.0.0.1',
        port: 7777,
        token: 'fixed-token',
        rootId: 'root-test0000000000000000000000',
      );
      await savePairing(oldConfig, support: support);
      final pairedDir = Directory(
        sync_bridge.pairedRootDirFor(
          support: support.path,
          rootId: 'root-test0000000000000000000000',
        ),
      )..createSync(recursive: true);
      final builtEndpoints = <String>[];

      await tester.pumpWidget(
        MaterialApp(
          home: PickerScreen(
            root: phoneRoot.path,
            supportDir: support,
            currentThemeId: 'dark',
            onSetTheme: (_) async {},
            buildClient: (_) => FakeServerClient(
              versionReply: minServerVersion,
              rootIdReply: 'root-test0000000000000000000000',
            ),
            buildSyncPort: (pairing, _) {
              builtEndpoints.add('${pairing.host}:${pairing.port}');
              return FakeSyncPort(rootId: 'root-test0000000000000000000000', rootDir: pairedDir.path);
            },
          ),
        ),
      );
      await settlePicker(tester);
      expect(builtEndpoints, ['127.0.0.1:7777']);

      await tester.tap(find.byIcon(Icons.menu));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Connected devices'));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const ValueKey('pairing-url-field')),
        'http://127.0.0.1:8888/?token=fixed-token',
      );
      await tester.tap(find.text('Pair'));
      await tester.pumpAndSettle();

      expect(readActivePairing(support)?.port, 8888);
      expect(
        builtEndpoints,
        ['127.0.0.1:7777', '127.0.0.1:8888'],
        reason:
            'host, port, and scheme belong to the active pairing just as the '
            'token does; keeping the old port makes a successful re-pair '
            'continue dialing the failed endpoint',
      );
    },
  );

  testWidgets(
    'unpairing removes the active sync controller and its unread status',
    (tester) async {
      final support = tempDir('alix-round5-unpair-controller-support-');
      final phoneRoot = tempDir('alix-round5-unpair-controller-phone-');
      final pairedDir = await pairedRoot(support);
      final port = FakeSyncPort(rootId: 'root-test0000000000000000000000', rootDir: pairedDir.path);
      port.tidyRenamedImpl = (_) => const [
        SyncRenamedEntry(old: 'A', new_: 'B'),
      ];

      await tester.pumpWidget(
        MaterialApp(
          home: PickerScreen(
            root: phoneRoot.path,
            supportDir: support,
            currentThemeId: 'dark',
            onSetTheme: (_) async {},
            buildClient: (_) => FakeServerClient(
              versionReply: minServerVersion,
              rootIdReply: 'root-test0000000000000000000000',
            ),
            buildSyncPort: (_, _) => port,
          ),
        ),
      );
      await settlePicker(tester);
      expect(find.byKey(const Key('sync-status')), findsOneWidget);

      await tester.tap(find.byIcon(Icons.menu));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Connected devices'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Unpair'));
      await tester.pumpAndSettle();
      await tester.pageBack();
      await tester.pumpAndSettle();

      expect(readActivePairing(support), isNull);
      expect(
        find.byKey(const Key('sync-status')),
        findsNothing,
        reason:
            'the unpaired screen must detach and dispose the old controller; '
            'otherwise its listener, status, and transport remain live',
      );
    },
  );
}
