import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/server_client.dart' show PairingExpired;
import 'package:alix_mobile/sync/sync_controller.dart';
import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync/sync_port.dart';
import 'package:alix_mobile/sync_client.dart';

import '../support/fake_sync_port.dart';

void main() {
  late Directory scratch;

  setUp(() {
    scratch = Directory.systemTemp.createTempSync('alix-sync-controller-');
  });

  tearDown(() {
    if (scratch.existsSync()) scratch.deleteSync(recursive: true);
  });

  File document(String name, [String content = '{}']) {
    final file = File('${scratch.path}/$name');
    file.writeAsStringSync(content);
    return file;
  }

  SyncPushPlanItem item(String deckId, {String entry = 'Biology', int? base}) {
    return SyncPushPlanItem(
      deckId: deckId,
      entry: entry,
      document: document('$deckId.json').path,
      base: base,
      phoneRevision: 1,
    );
  }

  group('cycle', () {
    test(
      'an entries answer naming another root aborts before any push',
      () async {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.entriesImpl = () async =>
            const SyncEntries(rootId: 'root-b', entries: []);
        port.planPushesImpl = () =>
            throw StateError('must not plan pushes after a root mismatch');
        final controller = SyncController(port: port);

        await controller.cycle();

        expect(controller.lastReport?.error, syncRootMismatchMessage);
        expect(port.pushCalls, isEmpty);
      },
    );

    test('pushes every planned item in order, recording each result', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.planPushesImpl = () => [
        item('deck-a'),
        item('deck-b'),
        item('deck-c'),
      ];
      port.pushImpl = (deckId, document, pulledRevision) async {
        return switch (deckId) {
          'deck-a' => SyncPushAccepted(deckId: deckId, revision: 2),
          'deck-b' => SyncPushConflict(deckId: deckId, desktopRevision: 5),
          _ => const SyncPushNotServed(),
        };
      };
      final controller = SyncController(port: port);

      await controller.cycle();

      expect(port.pushCalls, ['deck-a', 'deck-b', 'deck-c']);
      expect(port.recordPushCalls, hasLength(3));
      expect(port.recordPushCalls[0].$2, isA<SyncPushAcceptedOutcome>());
      expect(port.recordPushCalls[1].$2, isA<SyncPushConflictOutcome>());
      expect(port.recordPushCalls[2].$2, isA<SyncPushNotServedOutcome>());
      expect(controller.lastReport?.conflicts, ['Biology/deck-b']);
      expect(controller.lastReport?.error, isNull);
    });

    test('a pairing-expired push ends the cycle', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.planPushesImpl = () => [item('deck-a')];
      port.pushImpl = (_, _, _) async => throw const PairingExpired();
      final controller = SyncController(port: port);

      await controller.cycle();

      expect(controller.lastReport?.error, syncPairingExpiredMessage);
    });

    test(
      'a pull refused for free space names the two byte counts and continues',
      () async {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.entriesImpl = () async => const SyncEntries(
          rootId: 'root-a',
          entries: [
            SyncEntry(
              name: 'Biology',
              kind: 'workspace',
              members: 1,
              unpackedBytes: 500,
              leftOut: [],
            ),
          ],
        );
        port.pairedEntriesImpl = () => const [
          SyncEntryState(entry: 'Biology', kind: 'workspace', decks: []),
        ];
        port.pullImpl = (entry, target, unpackedBytes) async {
          throw const SyncFreeSpaceRefusal(needed: 2000, free: 100);
        };
        final controller = SyncController(port: port);

        await controller.cycle();

        expect(controller.lastReport?.refused, [
          'Biology: needs 2.0 KB, 100 bytes free',
        ]);
        expect(controller.lastReport?.error, isNull);
      },
    );

    test('a pull transport failure ends the cycle naming the entry', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-a',
        entries: [
          SyncEntry(
            name: 'Biology',
            kind: 'workspace',
            members: 1,
            unpackedBytes: 10,
            leftOut: [],
          ),
        ],
      );
      port.pairedEntriesImpl = () => const [
        SyncEntryState(entry: 'Biology', kind: 'workspace', decks: []),
      ];
      port.pullImpl = (entry, target, unpackedBytes) async {
        throw const SyncTransportFailure(500, 'boom');
      };
      final controller = SyncController(port: port);

      await controller.cycle();

      expect(controller.lastReport?.error, contains('Biology'));
      expect(controller.lastReport?.error, contains('500'));
    });

    test('a tapped entry pulls only that entry', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-a',
        entries: [
          SyncEntry(
            name: 'Biology',
            kind: 'workspace',
            members: 1,
            unpackedBytes: 10,
            leftOut: [],
          ),
          SyncEntry(
            name: 'Chemistry',
            kind: 'workspace',
            members: 1,
            unpackedBytes: 10,
            leftOut: [],
          ),
        ],
      );
      port.pairedEntriesImpl = () => const [
        SyncEntryState(entry: 'Biology', kind: 'workspace', decks: []),
        SyncEntryState(entry: 'Chemistry', kind: 'workspace', decks: []),
      ];

      final controller = SyncController(port: port);
      await controller.cycle(entry: 'Biology');

      expect(port.pullCalls, ['Biology']);
    });

    test(
      'orphaned names an entry the phone tracks that the desktop no longer serves',
      () async {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.entriesImpl = () async =>
            const SyncEntries(rootId: 'root-a', entries: []);
        port.pairedEntriesImpl = () => const [
          SyncEntryState(entry: 'Gone', kind: 'workspace', decks: []),
        ];
        final controller = SyncController(port: port);

        await controller.cycle();

        expect(controller.lastReport?.orphaned, ['Gone']);
      },
    );

    test('notOnPhone names a desktop entry the phone has never pulled', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-a',
        entries: [
          SyncEntry(
            name: 'New Deck',
            kind: 'deck',
            members: 1,
            unpackedBytes: 10,
            leftOut: [],
          ),
        ],
      );
      final controller = SyncController(port: port);

      await controller.cycle();

      expect(controller.lastReport?.notOnPhone, ['New Deck']);
      expect(controller.lastReport?.leftOut, isEmpty);
    });

    test(
      'leftOut and notOnPhone split correctly: an entry on the phone '
      'carries its own left_out members, an entry not on the phone is '
      'named in notOnPhone instead',
      () async {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.entriesImpl = () async => const SyncEntries(
          rootId: 'root-a',
          entries: [
            SyncEntry(
              name: 'Biology',
              kind: 'workspace',
              members: 2,
              unpackedBytes: 10,
              leftOut: ['decks/broken.md'],
            ),
            SyncEntry(
              name: 'Chemistry',
              kind: 'workspace',
              members: 1,
              unpackedBytes: 10,
              leftOut: [],
            ),
          ],
        );
        port.pairedEntriesImpl = () => const [
          SyncEntryState(entry: 'Biology', kind: 'workspace', decks: []),
        ];
        final controller = SyncController(port: port);

        await controller.cycle();

        expect(controller.lastReport?.leftOut, ['Biology/decks/broken.md']);
        expect(controller.lastReport?.notOnPhone, ['Chemistry']);
      },
    );

    test('renamed reports the tidy pairs from the lib', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.tidyRenamedImpl = (listed) => const [
        SyncRenamedEntry(old: 'Old Name', new_: 'New Name'),
      ];
      final controller = SyncController(port: port);

      await controller.cycle();

      expect(controller.lastReport?.renamed, ['Old Name → New Name']);
    });

    test('a cycle already running is left alone', () async {
      final gate = Completer<void>();
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.entriesImpl = () async {
        await gate.future;
        return const SyncEntries(rootId: 'root-a', entries: []);
      };
      final controller = SyncController(port: port);

      final first = controller.cycle();
      final second = controller.cycle();
      expect(controller.running, isTrue);
      gate.complete();
      await first;
      await second;

      expect(port.entriesCalls, [0]);
    });
  });

  group('pushOne', () {
    test('does nothing when nothing is planned for the deck', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      final controller = SyncController(port: port);

      await controller.pushOne('deck-a');

      expect(port.pushCalls, isEmpty);
    });

    test('is silent on a transport failure', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.planPushesImpl = () => [item('deck-a')];
      port.pushImpl = (_, _, _) async => throw const SyncTransportFailure(500, 'x');
      final controller = SyncController(port: port);

      await controller.pushOne('deck-a');

      expect(port.recordPushCalls, isEmpty);
    });

    test('a conflict is recorded and then visible through pendingConflicts', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.planPushesImpl = () => [item('deck-a')];
      port.pushImpl = (deckId, _, _) async => SyncPushConflict(
        deckId: deckId,
        desktopRevision: 4,
        desktopWriter: const SyncWriter(device: 'desk-1', atMs: 0),
      );
      port.pairedEntriesImpl = () => const [
        SyncEntryState(
          entry: 'Biology',
          kind: 'workspace',
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
      final controller = SyncController(port: port);

      await controller.pushOne('deck-a');

      expect(controller.pendingConflicts, hasLength(1));
      expect(controller.pendingConflicts.single.deckId, 'deck-a');
    });
  });

  group('resolve', () {
    test('keep-phone pushes with the returned item', () async {
      final pushItem = item('deck-a', base: 3);
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.resolveConflictImpl = (deckId, keepPhone) =>
          SyncResolutionPush(pushItem);
      final controller = SyncController(port: port);

      await controller.resolve('deck-a', keepPhone: true);

      expect(port.resolveConflictCalls, ['deck-a']);
      expect(port.pushCalls, ['deck-a']);
    });

    test('take-desktop pulls the returned entry', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.resolveConflictImpl = (deckId, keepPhone) =>
          const SyncResolutionPull('Biology');
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-a',
        entries: [
          SyncEntry(
            name: 'Biology',
            kind: 'workspace',
            members: 1,
            unpackedBytes: 10,
            leftOut: [],
          ),
        ],
      );
      final controller = SyncController(port: port);

      await controller.resolve('deck-a', keepPhone: false);

      expect(port.pullCalls, ['Biology']);
    });

    test('done refreshes without pushing or pulling', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.resolveConflictImpl = (deckId, keepPhone) =>
          const SyncResolutionDone();
      final controller = SyncController(port: port);
      var notifications = 0;
      controller.addListener(() => notifications++);

      await controller.resolve('deck-a', keepPhone: false);

      expect(port.pushCalls, isEmpty);
      expect(port.pullCalls, isEmpty);
      expect(notifications, greaterThan(0));
    });
  });

  group('removeOrphan', () {
    test('removes the entry through the port', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      final controller = SyncController(port: port);

      await controller.removeOrphan('Old Deck');

      expect(port.removeEntryCalls, ['Old Deck']);
    });
  });

  group('availableEntries', () {
    test('empty before any listing', () {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      final controller = SyncController(port: port);

      expect(controller.availableEntries, isEmpty);
    });

    test('lists a desktop entry the phone has no manifest for', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-a',
        entries: [
          SyncEntry(
            name: 'Biology',
            kind: 'workspace',
            members: 1,
            unpackedBytes: 10,
            leftOut: [],
          ),
        ],
      );
      final controller = SyncController(port: port);

      await controller.cycle();

      expect(controller.availableEntries.map((e) => e.name), ['Biology']);
    });

    test(
      'a first pull lands the entry, and the row is gone from the next read',
      () async {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.entriesImpl = () async => const SyncEntries(
          rootId: 'root-a',
          entries: [
            SyncEntry(
              name: 'Biology',
              kind: 'workspace',
              members: 1,
              unpackedBytes: 10,
              leftOut: [],
            ),
          ],
        );
        var pulled = false;
        port.pairedEntriesImpl = () => pulled
            ? const [SyncEntryState(entry: 'Biology', kind: 'workspace', decks: [])]
            : const [];
        port.applyPullImpl = (entry, zipPath) async {
          pulled = true;
          return const SyncPullReport(
            entry: 'Biology',
            kind: 'workspace',
            landed: [],
            kept: [],
            conflicts: [],
            phoneOnly: [],
            removed: [],
          );
        };
        final controller = SyncController(port: port);

        await controller.cycle();
        expect(controller.availableEntries.map((e) => e.name), ['Biology']);

        await controller.cycle(entry: 'Biology');
        expect(controller.availableEntries, isEmpty);
      },
    );
  });

  group('statusLine', () {
    test('names the tapped entry while running, then the report', () async {
      final gate = Completer<void>();
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.entriesImpl = () async {
        await gate.future;
        return const SyncEntries(rootId: 'root-a', entries: []);
      };
      final controller = SyncController(port: port);

      final future = controller.cycle(entry: 'Biology');
      expect(controller.statusLine, 'Syncing Biology…');
      gate.complete();
      await future;

      expect(controller.statusLine, 'Synced: up to date');
      controller.markReportRead();
      expect(controller.statusLine, isNull);
    });
  });
}
