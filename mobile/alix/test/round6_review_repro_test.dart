import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/sync/sync_controller.dart';
import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync_client.dart';

import 'support/fake_sync_port.dart';

void main() {
  Directory tempDir(String prefix) {
    final dir = Directory.systemTemp.createTempSync(prefix);
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  SyncPushPlanItem item(Directory scratch, String deckId) {
    final document = File('${scratch.path}/$deckId.json')
      ..writeAsStringSync('{}');
    return SyncPushPlanItem(
      deckId: deckId,
      entry: 'Biology',
      document: document.path,
      base: 1,
      phoneRevision: 2,
    );
  }

  List<SyncEntryState> pendingConflict() => const [
    SyncEntryState(
      entry: 'Biology',
      kind: 'workspace',
      digest: 'xxh64-0000000000000001',
      decks: [
        SyncDeckState(
          deckId: 'deck-a',
          path: 'decks/a.md',
          unpushed: true,
          conflict: PairedConflictPull(pulledRevision: 1),
        ),
      ],
    ),
  ];

  test(
    'take-desktop refuses to pull an entry listed by another root',
    () async {
      final scratch = tempDir('alix-round6-resolve-root-');
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.pairedEntriesImpl = pendingConflict;
      port.resolveConflictImpl = (_, _) => const SyncResolutionPull('Biology');
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-b',
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
      final controller = SyncController(port: port);

      await controller.resolve('deck-a', keepPhone: false);

      expect(
        port.pullCalls,
        isEmpty,
        reason:
            'a conflict choice must apply only bytes served by the paired root',
      );
      expect(
        controller.lastReport?.error,
        syncRootMismatchMessage('root-b', 'root-a'),
      );
    },
  );

  test(
    'take-desktop reports when the resolved entry is no longer served',
    () async {
      final scratch = tempDir('alix-round6-resolve-missing-');
      var conflictConsumed = false;
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.pairedEntriesImpl = () =>
          conflictConsumed ? const [] : pendingConflict();
      port.resolveConflictImpl = (_, _) {
        conflictConsumed = true;
        return const SyncResolutionPull('Biology');
      };
      port.entriesImpl = () async =>
          const SyncEntries(rootId: 'root-a', entries: []);
      final controller = SyncController(port: port);

      await controller.resolve('deck-a', keepPhone: false);

      expect(
        controller.lastReport?.error,
        isNotNull,
        reason:
            'a disappeared entry must not consume the choice as a silent success',
      );
    },
  );

  test('pushOne contains an unexpected planning exception', () async {
    final scratch = tempDir('alix-round6-plan-error-');
    final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
    port.planPushesImpl = () => throw StateError('state file corrupt');
    final controller = SyncController(port: port);

    await expectLater(
      controller.pushOne('deck-a'),
      completes,
      reason: 'pushOne promises silence on failure, including local planning',
    );
  });

  test('resolve contains an unexpected local resolution exception', () async {
    final scratch = tempDir('alix-round6-resolve-error-');
    final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
    port.pairedEntriesImpl = pendingConflict;
    port.resolveConflictImpl = (_, _) =>
        throw StateError('conflict state corrupt');
    final controller = SyncController(port: port);

    await expectLater(
      controller.resolve('deck-a', keepPhone: true),
      completes,
      reason:
          'an exception from the local conflict resolver must not escape its tap',
    );
  });

  test(
    'a planning exception in the deferred drain cannot brick the cycle',
    () async {
      final scratch = tempDir('alix-round6-drain-error-');
      final pullStarted = Completer<void>();
      final finishPull = Completer<void>();
      var planCalls = 0;
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
      port.planPushesImpl = () {
        planCalls++;
        if (planCalls == 1) return const <SyncPushPlanItem>[];
        throw StateError('state file corrupt');
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
      final summaryPush = controller.pushOne('deck-a');
      finishPull.complete();

      Object? cycleError;
      try {
        await cycle;
      } on Object catch (error) {
        cycleError = error;
      }
      await summaryPush;

      expect(
        (cycleError, controller.running),
        (null, false),
        reason:
            'the cycle must contain the error and release its single-flight '
            'owner so later cycles can run',
      );
    },
  );

  test('a summary push at the drain tail is attempted, not stranded', () async {
    final scratch = tempDir('alix-round6-drain-tail-');
    var planCalls = 0;
    var scheduled = false;
    var queuedCompleted = false;
    Future<void>? queuedPush;
    late SyncController controller;
    final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
    port.planPushesImpl = () {
      planCalls++;
      return planCalls == 1
          ? const <SyncPushPlanItem>[]
          : [item(scratch, 'deck-a')];
    };
    port.pairedOrphansImpl = (_) {
      if (!scheduled) {
        scheduled = true;
        scheduleMicrotask(() {
          scheduleMicrotask(() {
            queuedPush = controller.pushOne('deck-a');
            queuedPush!.then((_) => queuedCompleted = true);
          });
        });
      }
      return const [];
    };
    controller = SyncController(port: port);

    await controller.cycle();
    await Future<void>.delayed(Duration.zero);

    expect(queuedPush, isNotNull);
    expect(
      port.pushCalls,
      ['deck-a'],
      reason:
          'the transition from draining to idle must not have a queue-only gap',
    );
    expect(
      queuedCompleted,
      isTrue,
      reason: 'a post-review caller must never wait on an abandoned completer',
    );
  });
}
