import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/server_client.dart' show PairingExpired;
import 'package:alix_mobile/sync/sync_controller.dart';
import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync/sync_port.dart';
import 'package:alix_mobile/sync_client.dart';

import 'support/fake_sync_port.dart';

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
    SyncEntry listed(String name, {int unpackedBytes = 10}) => SyncEntry(
      name: name,
      kind: 'workspace',
      members: 1,
      unpackedBytes: unpackedBytes,
      fingerprint: '0000000000002',
      leftOut: const [],
    );

    FakeSyncPort freshPhone(List<SyncEntry> desktop) {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      final onPhone = <SyncEntryState>[];
      port.entriesImpl = () async =>
          SyncEntries(rootId: 'root-a', entries: desktop);
      port.pairedEntriesImpl = () => List.of(onPhone);
      port.applyPullImpl = (entry, zipPath) async {
        onPhone.add(
          SyncEntryState(
            entry: entry,
            kind: 'workspace',
            fingerprint: '0000000000002',
            decks: const [],
          ),
        );
        return SyncPullReport(
          entry: entry,
          kind: 'workspace',
          landed: ['deck-$entry'],
          kept: const [],
          conflicts: const [],
          phoneOnly: const [],
          removed: const [],
        );
      };
      return port;
    }

    test(
      'SYNC on a fresh phone lands every workspace the desktop lists',
      () async {
        final port = freshPhone([
          listed('Biology'),
          listed('Chemistry'),
          listed('Physics'),
        ]);
        final controller = SyncController(port: port);

        await controller.cycle(everyListed: true);

        expect(port.pullCalls, ['Biology', 'Chemistry', 'Physics']);
        expect(port.applyPullCalls, ['Biology', 'Chemistry', 'Physics']);
        expect(controller.lastReport?.landed, hasLength(3));
        expect(controller.lastReport?.notOnPhone, isEmpty);
        expect(controller.lastReport?.error, isNull);
      },
    );

    test(
      'tapping one listed row on a fresh phone pulls only that row',
      () async {
        final port = freshPhone([listed('Biology'), listed('Chemistry')]);
        final controller = SyncController(port: port);

        await controller.cycle(entry: 'Chemistry');

        expect(port.pullCalls, ['Chemistry']);
        expect(controller.lastReport?.notOnPhone, ['Biology']);
      },
    );

    test('SYNC on a fresh phone reports a free-space refusal and lands the '
        'rest', () async {
      final port = freshPhone([
        listed('Biology'),
        listed('Huge', unpackedBytes: 5000),
        listed('Physics'),
      ]);
      port.pullImpl = (entry, target, unpackedBytes) async {
        if (entry == 'Huge') {
          throw const SyncFreeSpaceRefusal(needed: 5000, free: 100);
        }
        await target.writeAsBytes(const []);
        return 0;
      };
      final controller = SyncController(port: port);

      await controller.cycle(everyListed: true);

      expect(port.applyPullCalls, ['Biology', 'Physics']);
      expect(controller.lastReport?.refused, [
        'Huge: needs 4.9 KB, 100 bytes free',
      ]);
      expect(controller.lastReport?.notOnPhone, ['Huge']);
    });

    test(
      'SYNC continues with later listed workspaces after one apply fails',
      () async {
        final port = freshPhone([
          listed('Biology'),
          listed('Chemistry'),
          listed('Physics'),
        ]);
        port.applyPullImpl = (entry, zipPath) async {
          if (entry == 'Chemistry') throw Exception('bad archive');
          return SyncPullReport(
            entry: entry,
            kind: 'workspace',
            landed: ['deck-$entry'],
            kept: const [],
            conflicts: const [],
            phoneOnly: const [],
            removed: const [],
          );
        };
        final controller = SyncController(port: port);

        await controller.cycle(everyListed: true);

        expect(
          port.applyPullCalls,
          ['Biology', 'Chemistry', 'Physics'],
          reason:
              'one entry-specific apply failure must not skip later entries',
        );
        expect(controller.lastReport?.landed, [
          'Biology/deck-Biology',
          'Physics/deck-Physics',
        ]);
        expect(controller.lastReport?.error, contains('Chemistry'));
      },
    );

    test('the launch cycle on a fresh phone pulls nothing: only SYNC '
        'fetches entries not yet on the phone', () async {
      final port = freshPhone([listed('Biology'), listed('Chemistry')]);
      final controller = SyncController(port: port);

      await controller.cycle();

      expect(port.pullCalls, isEmpty);
      expect(controller.lastReport?.notOnPhone, ['Biology', 'Chemistry']);
    });

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

        expect(
          controller.lastReport?.error,
          syncRootMismatchMessage('root-b', 'root-a'),
        );
        expect(port.pushCalls, isEmpty);
      },
    );

    test('an entries answer naming another root is never exposed as this '
        "pairing's available entries", () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-b',
        entries: [
          SyncEntry(
            name: 'Other Root Deck.md',
            kind: 'deck',
            members: 1,
            unpackedBytes: 10,
            fingerprint: '0000000000002',
            leftOut: [],
          ),
        ],
      );
      final controller = SyncController(port: port);

      await controller.cycle();

      expect(
        controller.lastReport?.error,
        syncRootMismatchMessage('root-b', 'root-a'),
      );
      expect(
        controller.availableEntries,
        isEmpty,
        reason: 'a refused root must not populate actionable picker rows',
      );
    });

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

    test('a push the desktop refuses with a SyncFailureDto is reported with its remedy', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.planPushesImpl = () => [item('deck-a')];
      port.pushImpl = (_, _, _) async => const SyncPushRejected(
        status: 500,
        body: '{"class":"damaged-sync-state"}',
        failure: SyncServerFailure(
          kind: 'damaged-sync-state',
          message: 'cannot write the progress file of deck deck-a',
          remedy: 'Run `alix doctor` on the computer.',
        ),
      );
      final controller = SyncController(port: port);

      await controller.cycle();

      expect(controller.lastReport?.refused, [
        'Biology/deck-a: cannot write the progress file of deck deck-a\n'
        'Run `alix doctor` on the computer.',
      ]);
    });

    test('an accepted push is present in the cycle report', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.planPushesImpl = () => [item('deck-a')];
      final controller = SyncController(port: port);

      await controller.cycle();

      expect(port.pushCalls, ['deck-a']);
      expect(
        controller.reportUnread,
        isTrue,
        reason: 'the spec requires the sync report to list pushed decks',
      );
      expect(controller.statusLine, contains('pushed'));
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
              fingerprint: '0000000000002',
              leftOut: [],
            ),
          ],
        );
        port.pairedEntriesImpl = () => const [
          SyncEntryState(
            entry: 'Biology',
            kind: 'workspace',
            fingerprint: '0000000000001',
            decks: [],
          ),
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
            fingerprint: '0000000000002',
            leftOut: [],
          ),
        ],
      );
      port.pairedEntriesImpl = () => const [
        SyncEntryState(
          entry: 'Biology',
          kind: 'workspace',
          fingerprint: '0000000000001',
          decks: [],
        ),
      ];
      port.pullImpl = (entry, target, unpackedBytes) async {
        throw const SyncTransportFailure(500, 'boom');
      };
      final controller = SyncController(port: port);

      await controller.cycle();

      expect(controller.lastReport?.error, contains('Biology'));
      expect(controller.lastReport?.error, contains('500'));
    });

    test('a pull failure the desktop explains shows its message and remedy', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-a',
        entries: [
          SyncEntry(
            name: 'Biology',
            kind: 'workspace',
            members: 1,
            unpackedBytes: 10,
            fingerprint: '0000000000002',
            leftOut: [],
          ),
        ],
      );
      port.pairedEntriesImpl = () => const [
        SyncEntryState(
          entry: 'Biology',
          kind: 'workspace',
          fingerprint: '0000000000001',
          decks: [],
        ),
      ];
      port.pullImpl = (entry, target, unpackedBytes) async {
        throw const SyncServerFailure(
          kind: 'damaged-sync-state',
          message: 'cannot write the staging folder',
          remedy: 'Run `alix doctor` on the computer.',
        );
      };
      final controller = SyncController(port: port);

      await controller.cycle();

      expect(
        controller.lastReport?.error,
        'could not pull Biology: cannot write the staging folder\n'
        'Run `alix doctor` on the computer.',
      );
    });

    test('a listing failure the desktop explains shows its message and remedy', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.entriesImpl = () async => throw const SyncServerFailure(
        kind: 'damaged-sync-state',
        message: 'cannot read .alix/sync.toml',
        remedy: 'Run `alix doctor` on the computer.',
      );
      final controller = SyncController(port: port);

      await controller.cycle();

      expect(
        controller.lastReport?.error,
        'could not list what the desktop serves: cannot read .alix/sync.toml\n'
        'Run `alix doctor` on the computer.',
      );
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
            fingerprint: '0000000000002',
            leftOut: [],
          ),
          SyncEntry(
            name: 'Chemistry',
            kind: 'workspace',
            members: 1,
            unpackedBytes: 10,
            fingerprint: '0000000000002',
            leftOut: [],
          ),
        ],
      );
      port.pairedEntriesImpl = () => const [
        SyncEntryState(
          entry: 'Biology',
          kind: 'workspace',
          fingerprint: '0000000000001',
          decks: [],
        ),
        SyncEntryState(
          entry: 'Chemistry',
          kind: 'workspace',
          fingerprint: '0000000000001',
          decks: [],
        ),
      ];

      final controller = SyncController(port: port);
      await controller.cycle(entry: 'Biology');

      expect(port.pullCalls, ['Biology']);
    });

    group('an entry whose fingerprint is the one the phone last pulled', () {
      const unchanged = '000000000005a';
      const moved = '000000000005v';
      const desktop = SyncEntries(
        rootId: 'root-a',
        entries: [
          SyncEntry(
            name: 'Biology',
            kind: 'workspace',
            members: 1,
            unpackedBytes: 10,
            fingerprint: unchanged,
            leftOut: [],
          ),
          SyncEntry(
            name: 'Chemistry',
            kind: 'workspace',
            members: 1,
            unpackedBytes: 10,
            fingerprint: moved,
            leftOut: [],
          ),
        ],
      );
      const onPhone = [
        SyncEntryState(
          entry: 'Biology',
          kind: 'workspace',
          fingerprint: unchanged,
          decks: [],
        ),
        SyncEntryState(
          entry: 'Chemistry',
          kind: 'workspace',
          fingerprint: '0000000000001',
          decks: [],
        ),
      ];

      test(
        'is skipped: the cycle pulls only the entry whose fingerprint moved',
        () async {
          final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
          port.entriesImpl = () async => desktop;
          port.pairedEntriesImpl = () => onPhone;

          await SyncController(port: port).cycle();

          expect(port.pullCalls, ['Chemistry']);
        },
      );

      test('is skipped when tapped, too', () async {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.entriesImpl = () async => desktop;
        port.pairedEntriesImpl = () => onPhone;

        await SyncController(port: port).cycle(entry: 'Biology');

        expect(port.pullCalls, isEmpty);
      });

      test('is skipped while one of its decks waits in conflict', () async {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.entriesImpl = () async => desktop;
        port.pairedEntriesImpl = () => const [
          SyncEntryState(
            entry: 'Biology',
            kind: 'workspace',
            fingerprint: unchanged,
            decks: [
              SyncDeckState(
                deckId: 'deck-cells',
                path: 'decks/cells.md',
                unpushed: true,
                conflict: PairedConflictPush(desktopRevision: 4),
              ),
            ],
          ),
        ];

        await SyncController(port: port).cycle();

        expect(port.pullCalls, isEmpty);
      });

      test('is pulled when a push landed in it this cycle', () async {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.entriesImpl = () async => desktop;
        port.pairedEntriesImpl = () => onPhone;
        port.planPushesImpl = () => [item('deck-cells')];

        await SyncController(port: port).cycle();

        expect(port.pushCalls, ['deck-cells']);
        expect(port.pullCalls, ['Biology', 'Chemistry']);
      });
    });

    test('a pull stages its zip at the path the lib names, not one the '
        'controller derives itself', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-a',
        entries: [
          SyncEntry(
            name: 'Biology',
            kind: 'workspace',
            members: 1,
            unpackedBytes: 10,
            fingerprint: '0000000000002',
            leftOut: [],
          ),
        ],
      );
      final namedZip = '${scratch.path}/lib-named-staging/Biology.zip';
      port.pairedStagingZipImpl = (entry) => namedZip;
      String? seenZipPath;
      port.applyPullImpl = (entry, zipPath) async {
        seenZipPath = zipPath;
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
      await controller.cycle(entry: 'Biology');

      expect(seenZipPath, namedZip);
    });

    test('a deck whose push conflicts and whose pull then conflicts is '
        'counted once', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.planPushesImpl = () => [item('deck-a')];
      port.pushImpl = (deckId, _, _) async =>
          SyncPushConflict(deckId: deckId, desktopRevision: 25);
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-a',
        entries: [
          SyncEntry(
            name: 'Biology',
            kind: 'workspace',
            members: 1,
            unpackedBytes: 10,
            fingerprint: '0000000000002',
            leftOut: [],
          ),
        ],
      );
      port.applyPullImpl = (entry, zipPath) async => const SyncPullReport(
        entry: 'Biology',
        kind: 'workspace',
        landed: [],
        kept: [],
        conflicts: ['deck-a'],
        phoneOnly: [],
        removed: [],
      );
      final controller = SyncController(port: port);

      await controller.cycle(entry: 'Biology');

      final report = controller.lastReport!;
      expect(report.conflicts, hasLength(1), reason: 'conflicts: ${report.conflicts}');
      expect(report.conflictDeckIds, ['deck-a'], reason: 'conflict ids');
      expect(report.summary(), contains('1 conflict'), reason: 'status line');
    });

    test('a deck whose unreadable phone progress the pull replaced is '
        'reported and marks the report unread', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-a',
        entries: [
          SyncEntry(
            name: 'Biology',
            kind: 'workspace',
            members: 1,
            unpackedBytes: 10,
            fingerprint: '0000000000002',
            leftOut: [],
          ),
        ],
      );
      port.applyPullImpl = (entry, zipPath) async => const SyncPullReport(
        entry: 'Biology',
        kind: 'workspace',
        landed: ['deck-a'],
        kept: [],
        conflicts: [],
        phoneOnly: [],
        removed: [],
        replaced: ['deck-a'],
      );
      final controller = SyncController(port: port);

      await controller.cycle(entry: 'Biology');

      final report = controller.lastReport!;
      expect(report.replaced, ['Biology/deck-a'], reason: 'report: replaced');
      expect(report.isEmpty, isFalse, reason: 'report: unread');
      expect(
        report.summary(),
        contains('1 replaced'),
        reason: 'status line: names the replacement',
      );
    });

    test('an orphan the lib reports is used as-is, not re-derived by the '
        'controller', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.entriesImpl = () async =>
          const SyncEntries(rootId: 'root-a', entries: []);
      port.pairedOrphansImpl = (listed) => const ['lib-named-orphan'];
      final controller = SyncController(port: port);

      await controller.cycle();

      expect(controller.lastReport?.orphaned, ['lib-named-orphan']);
    });

    test(
      'orphaned names an entry the phone tracks that the desktop no longer serves',
      () async {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.entriesImpl = () async =>
            const SyncEntries(rootId: 'root-a', entries: []);
        port.pairedEntriesImpl = () => const [
          SyncEntryState(
            entry: 'Gone',
            kind: 'workspace',
            fingerprint: '0000000000001',
            decks: [],
          ),
        ];
        final controller = SyncController(port: port);

        await controller.cycle();

        expect(controller.lastReport?.orphaned, ['Gone']);
      },
    );

    test(
      'notOnPhone names a desktop entry the phone has never pulled',
      () async {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.entriesImpl = () async => const SyncEntries(
          rootId: 'root-a',
          entries: [
            SyncEntry(
              name: 'New Deck',
              kind: 'deck',
              members: 1,
              unpackedBytes: 10,
              fingerprint: '0000000000002',
              leftOut: [],
            ),
          ],
        );
        final controller = SyncController(port: port);

        await controller.cycle();

        expect(controller.lastReport?.notOnPhone, ['New Deck']);
        expect(controller.lastReport?.leftOut, isEmpty);
      },
    );

    test('leftOut and notOnPhone split correctly: an entry on the phone '
        'carries its own left_out members, an entry not on the phone is '
        'named in notOnPhone instead', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-a',
        entries: [
          SyncEntry(
            name: 'Biology',
            kind: 'workspace',
            members: 2,
            unpackedBytes: 10,
            fingerprint: '0000000000002',
            leftOut: ['decks/broken.md'],
          ),
          SyncEntry(
            name: 'Chemistry',
            kind: 'workspace',
            members: 1,
            unpackedBytes: 10,
            fingerprint: '0000000000002',
            leftOut: [],
          ),
        ],
      );
      port.pairedEntriesImpl = () => const [
        SyncEntryState(
          entry: 'Biology',
          kind: 'workspace',
          fingerprint: '0000000000001',
          decks: [],
        ),
      ];
      final controller = SyncController(port: port);

      await controller.cycle();

      expect(controller.lastReport?.leftOut, ['Biology/decks/broken.md']);
      expect(controller.lastReport?.notOnPhone, ['Chemistry']);
    });

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

    test(
      'an apply failure ends the cycle and reports it, naming the entry',
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
              fingerprint: '0000000000002',
              leftOut: [],
            ),
          ],
        );
        port.pairedEntriesImpl = () => const [
          SyncEntryState(
            entry: 'Biology',
            kind: 'workspace',
            fingerprint: '0000000000001',
            decks: [],
          ),
        ];
        port.applyPullImpl = (entry, zipPath) async => throw Exception('boom');

        final controller = SyncController(port: port);
        await controller.cycle();

        expect(controller.running, isFalse);
        expect(controller.lastReport?.error, contains('Biology'));
      },
    );

    test('an apply failure discards the downloaded staging zip', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-a',
        entries: [
          SyncEntry(
            name: 'Biology',
            kind: 'workspace',
            members: 1,
            unpackedBytes: 10,
            fingerprint: '0000000000002',
            leftOut: [],
          ),
        ],
      );
      port.pairedEntriesImpl = () => const [
        SyncEntryState(
          entry: 'Biology',
          kind: 'workspace',
          fingerprint: '0000000000001',
          decks: [],
        ),
      ];
      final zip = File('${scratch.path}/staging/Biology.zip');
      port.pairedStagingZipImpl = (_) => zip.path;
      port.applyPullImpl = (entry, zipPath) async =>
          throw Exception('the pull names another root');
      final controller = SyncController(port: port);

      await controller.cycle();

      expect(controller.lastReport?.error, contains('Biology'));
      expect(
        zip.existsSync(),
        isFalse,
        reason: 'a discarded pull must not leave its private zip behind',
      );
    });

    test(
      'a cycle that changed nothing leaves reportUnread false, so the '
      'picker stays silent',
      () async {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        final controller = SyncController(port: port);

        await controller.cycle();

        expect(controller.lastReport?.isEmpty, isTrue);
        expect(controller.reportUnread, isFalse);
      },
    );

    test(
      'a cycle whose only content is a never-pulled desktop entry leaves '
      'reportUnread false too: the picker row already says so',
      () async {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.entriesImpl = () async => const SyncEntries(
          rootId: 'root-a',
          entries: [
            SyncEntry(
              name: 'New Deck',
              kind: 'deck',
              members: 1,
              unpackedBytes: 10,
              fingerprint: '0000000000002',
              leftOut: [],
            ),
          ],
        );
        final controller = SyncController(port: port);

        await controller.cycle();

        expect(controller.lastReport?.notOnPhone, ['New Deck']);
        expect(controller.lastReport?.isEmpty, isTrue);
        expect(controller.reportUnread, isFalse);
      },
    );
  });

  group('pushOne', () {
    test('does nothing when nothing is planned for the deck', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      final controller = SyncController(port: port);

      await controller.pushOne('deck-a');

      expect(port.pushCalls, isEmpty);
    });

    test(
      'a running cycle holds off a summary push for the same deck; the '
      'cycle pushes it once, the summary push does not double it',
      () async {
        final gate = Completer<void>();
        var alreadyPushed = false;
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.planPushesImpl = () =>
            alreadyPushed ? const [] : [item('deck-a')];
        port.pushImpl = (deckId, _, _) async {
          await gate.future;
          alreadyPushed = true;
          return SyncPushAccepted(deckId: deckId, revision: 1);
        };
        final controller = SyncController(port: port);

        final cycle = controller.cycle();
        final push = controller.pushOne('deck-a');
        gate.complete();
        await cycle;
        await push;

        expect(port.pushCalls, ['deck-a']);
      },
    );

    test('a summary push that becomes eligible after the running cycle planned '
        'its pushes is still attempted', () async {
      final pullStarted = Completer<void>();
      final finishPull = Completer<void>();
      var changedAfterPlanning = false;
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-a',
        entries: [
          SyncEntry(
            name: 'Biology',
            kind: 'workspace',
            members: 1,
            unpackedBytes: 10,
            fingerprint: '0000000000002',
            leftOut: [],
          ),
        ],
      );
      port.pairedEntriesImpl = () => const [
        SyncEntryState(
          entry: 'Biology',
          kind: 'workspace',
          fingerprint: '0000000000001',
          decks: [],
        ),
      ];
      port.planPushesImpl = () =>
          changedAfterPlanning ? [item('deck-a')] : const [];
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
      await push;
      await cycle;

      expect(
        port.pushCalls,
        ['deck-a'],
        reason:
            'finishing review after the cycle planned must not drop '
            'the summary trigger',
      );
    });

    test('is silent on a transport failure', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.planPushesImpl = () => [item('deck-a')];
      port.pushImpl = (_, _, _) async => throw const SyncTransportFailure(500, 'x');
      final controller = SyncController(port: port);

      await controller.pushOne('deck-a');

      expect(port.recordPushCalls, isEmpty);
    });

    test(
      'an unexpected exception (a closed port mid-attempt, say) does not '
      'escape the returned future',
      () async {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.planPushesImpl = () => [item('deck-a')];
        port.pushImpl = (_, _, _) async => throw StateError('boom');
        final controller = SyncController(port: port);

        await expectLater(controller.pushOne('deck-a'), completes);
      },
    );

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
          fingerprint: '0000000000001',
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

  group('deck labels', () {
    List<SyncEntryState> looseDeckConflict() => const [
      SyncEntryState(
        entry: 'greek.md',
        kind: 'deck',
        fingerprint: '0000000000001',
        decks: [
          SyncDeckState(
            deckId: 'deck-1',
            path: 'greek.md',
            unpushed: false,
            conflict: PairedConflictPush(desktopRevision: 2),
          ),
        ],
      ),
    ];

    test(
      'a loose deck in conflict is labeled by its title, never doubled '
      'as entry/entry',
      () {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.pairedEntriesImpl = looseDeckConflict;
        port.deckTitleImpl = (path) => path == 'greek.md' ? 'Greek' : null;
        final controller = SyncController(port: port);

        expect(controller.pendingConflicts.single.label, 'Greek');
      },
    );

    test(
      'a loose deck the phone holds no local copy of falls back to the '
      'entry name, never entry/entry',
      () {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.pairedEntriesImpl = looseDeckConflict;
        final controller = SyncController(port: port);

        expect(controller.pendingConflicts.single.label, 'greek.md');
      },
    );

    test(
      'a workspace member still reads entry/basename, unaffected by the '
      'deck-title lookup',
      () {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.pairedEntriesImpl = () => const [
          SyncEntryState(
            entry: 'Biology',
            kind: 'workspace',
            fingerprint: '0000000000001',
            decks: [
              SyncDeckState(
                deckId: 'deck-1',
                path: 'decks/cells.md',
                unpushed: false,
                conflict: PairedConflictPush(desktopRevision: 2),
              ),
            ],
          ),
        ];
        port.deckTitleImpl = (path) => 'must not be used for a workspace';
        final controller = SyncController(port: port);

        expect(controller.pendingConflicts.single.label, 'Biology/cells.md');
      },
    );
  });

  group('resolve', () {
    List<SyncEntryState> pendingConflictFor(String deckId) => [
      SyncEntryState(
        entry: 'Biology',
        kind: 'workspace',
        fingerprint: '0000000000001',
        decks: [
          SyncDeckState(
            deckId: deckId,
            path: 'decks/a.md',
            unpushed: true,
            conflict: const PairedConflictPush(desktopRevision: 4),
          ),
        ],
      ),
    ];

    test('keep-phone pushes with the returned item', () async {
      final pushItem = item('deck-a', base: 3);
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.pairedEntriesImpl = () => pendingConflictFor('deck-a');
      port.resolveConflictImpl = (deckId, keepPhone) =>
          SyncResolutionPush(pushItem);
      final controller = SyncController(port: port);

      await controller.resolve('deck-a', keepPhone: true);

      expect(port.resolveConflictCalls, ['deck-a']);
      expect(port.pushCalls, ['deck-a']);
    });

    test(
      'keep-phone silently swallows an unexpected exception, matching '
      'pushOne',
      () async {
        final pushItem = item('deck-a', base: 3);
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.pairedEntriesImpl = () => pendingConflictFor('deck-a');
        port.resolveConflictImpl = (deckId, keepPhone) =>
            SyncResolutionPush(pushItem);
        port.pushImpl = (_, _, _) async => throw StateError('boom');
        final controller = SyncController(port: port);

        await expectLater(
          controller.resolve('deck-a', keepPhone: true),
          completes,
        );
      },
    );

    test('take-desktop pulls the returned entry', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.pairedEntriesImpl = () => pendingConflictFor('deck-a');
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
            fingerprint: '0000000000002',
            leftOut: [],
          ),
        ],
      );
      final controller = SyncController(port: port);

      await controller.resolve('deck-a', keepPhone: false);

      expect(port.pullCalls, ['Biology']);
    });

    test('take-desktop reports an apply failure instead of leaking it from '
        'the conflict action', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.pairedEntriesImpl = () => pendingConflictFor('deck-a');
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
            fingerprint: '0000000000002',
            leftOut: [],
          ),
        ],
      );
      port.applyPullImpl = (entry, zipPath) async =>
          throw Exception('the pull names root `root-b`');
      final controller = SyncController(port: port);

      await expectLater(
        controller.resolve('deck-a', keepPhone: false),
        completes,
        reason: 'a failed choice must stay in the report, not escape its tap',
      );

      expect(controller.lastReport?.error, contains('root-b'));
    });

    test(
      'take-desktop reports an unexpected exception (a StateError) '
      'instead of leaking it from the conflict action',
      () async {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.pairedEntriesImpl = () => pendingConflictFor('deck-a');
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
              fingerprint: '0000000000002',
              leftOut: [],
            ),
          ],
        );
        port.applyPullImpl = (entry, zipPath) async => throw StateError('boom');
        final controller = SyncController(port: port);

        await expectLater(
          controller.resolve('deck-a', keepPhone: false),
          completes,
          reason: 'a failed choice must stay in the report, not escape its tap',
        );

        expect(controller.lastReport?.error, contains('boom'));
      },
    );

    test('done refreshes without pushing or pulling', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.pairedEntriesImpl = () => pendingConflictFor('deck-a');
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

    test(
      'resolving a conflict updates the stale status line behind the '
      'sheet at once, rather than waiting for the sheet to close',
      () async {
        var conflicted = true;
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.planPushesImpl = () => [item('deck-a')];
        port.pushImpl = (deckId, _, _) async => SyncPushConflict(
          deckId: deckId,
          desktopRevision: 4,
          desktopWriter: const SyncWriter(device: 'desk-1', atMs: 0),
        );
        port.pairedEntriesImpl = () =>
            conflicted ? pendingConflictFor('deck-a') : const [];
        final controller = SyncController(port: port);

        await controller.cycle();
        expect(controller.statusLine, contains('1 conflict'));

        port.resolveConflictImpl = (deckId, keepPhone) {
          conflicted = false;
          return const SyncResolutionDone();
        };
        await controller.resolve('deck-a', keepPhone: true);

        expect(controller.statusLine, isNot(contains('conflict')));
      },
    );

    test(
      'a resolve for a deck with no pending conflict is a silent no-op, '
      'so a second tap on an already-resolved conflict cannot throw',
      () async {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        final controller = SyncController(port: port);
        var notifications = 0;
        controller.addListener(() => notifications++);

        await controller.resolve('deck-a', keepPhone: true);

        expect(port.resolveConflictCalls, isEmpty);
        expect(notifications, 0);
      },
    );

    test(
      'a local resolver failure before a resolution exists reports the '
      'error and leaves the conflict pending',
      () async {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.pairedEntriesImpl = () => pendingConflictFor('deck-a');
        port.resolveConflictImpl = (_, _) =>
            throw StateError('conflict state corrupt');
        final controller = SyncController(port: port);

        await controller.resolve('deck-a', keepPhone: true);

        expect(controller.lastReport?.error, contains('conflict state corrupt'));
        expect(
          controller.pendingConflicts.map((c) => c.deckId),
          contains('deck-a'),
          reason: 'nothing was written, so the conflict must still be there',
        );
      },
    );

    test(
      'a take-desktop refusal recovers on the next cycle once the '
      'listing is correct',
      () async {
        var conflictConsumed = false;
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        port.pairedEntriesImpl = () => conflictConsumed
            ? const [
                SyncEntryState(
                  entry: 'Biology',
                  kind: 'workspace',
                  fingerprint: '0000000000001',
                  decks: [],
                ),
              ]
            : pendingConflictFor('deck-a');
        port.resolveConflictImpl = (_, _) {
          conflictConsumed = true;
          return const SyncResolutionPull('Biology');
        };
        port.entriesImpl = () async => const SyncEntries(
          rootId: 'root-b',
          entries: [
            SyncEntry(
              name: 'Biology',
              kind: 'workspace',
              members: 1,
              unpackedBytes: 10,
              fingerprint: '0000000000002',
              leftOut: [],
            ),
          ],
        );
        final controller = SyncController(port: port);

        await controller.resolve('deck-a', keepPhone: false);

        expect(port.pullCalls, isEmpty);
        expect(
          controller.lastReport?.error,
          syncRootMismatchMessage('root-b', 'root-a'),
        );
        expect(
          controller.pendingConflicts,
          isEmpty,
          reason: 'the choice was already consumed locally',
        );

        port.entriesImpl = () async => const SyncEntries(
          rootId: 'root-a',
          entries: [
            SyncEntry(
              name: 'Biology',
              kind: 'workspace',
              members: 1,
              unpackedBytes: 10,
              fingerprint: '0000000000002',
              leftOut: [],
            ),
          ],
        );
        var pulled = false;
        port.pullImpl = (entry, target, unpackedBytes) async {
          pulled = true;
          await target.writeAsBytes(const []);
          return 0;
        };

        await controller.cycle();

        expect(
          port.pullCalls,
          ['Biology'],
          reason:
              'a later cycle against a corrected listing must re-plan the '
              'pull the refusal held back',
        );
        expect(pulled, isTrue);
        expect(controller.pendingConflicts, isEmpty);
      },
    );
  });

  group('removeOrphan', () {
    test('removes the entry through the port', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      final controller = SyncController(port: port);

      await controller.removeOrphan('Old Deck');

      expect(port.removeEntryCalls, ['Old Deck']);
    });

    test('drops the entry from the last report, so the sheet reflects it '
        'without waiting for the next cycle', () async {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      port.entriesImpl = () async =>
          const SyncEntries(rootId: 'root-a', entries: []);
      port.pairedEntriesImpl = () => const [
        SyncEntryState(
          entry: 'Old Deck',
          kind: 'workspace',
          fingerprint: '0000000000001',
          decks: [],
        ),
      ];
      final controller = SyncController(port: port);
      await controller.cycle();
      expect(controller.lastReport?.orphaned, ['Old Deck']);

      await controller.removeOrphan('Old Deck');

      expect(controller.lastReport?.orphaned, isEmpty);
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
            fingerprint: '0000000000002',
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
              fingerprint: '0000000000002',
              leftOut: [],
            ),
          ],
        );
        var pulled = false;
        port.pairedEntriesImpl = () => pulled
            ? const [
                SyncEntryState(
                  entry: 'Biology',
                  kind: 'workspace',
                  fingerprint: '0000000000001',
                  decks: [],
                ),
              ]
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
      // A renamed pair makes the cycle non-empty (so reportUnread stays
      // true) without changing summary()'s text: only landed/conflicts/
      // refused/orphaned feed the summary line, so this keeps the wording
      // 'Synced: up to date' below while still exercising the report phase.
      port.tidyRenamedImpl = (listed) => const [
        SyncRenamedEntry(old: 'A', new_: 'B'),
      ];
      final controller = SyncController(port: port);

      final future = controller.cycle(entry: 'Biology');
      expect(controller.statusLine, 'Syncing Biology…');
      gate.complete();
      await future;

      expect(controller.statusLine, 'Synced: up to date');
      controller.markReportRead();
      expect(controller.statusLine, isNull);
    });

    test(
      'an empty cycle leaves the status line clear, never "up to date"',
      () async {
        final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
        final controller = SyncController(port: port);

        await controller.cycle();

        expect(controller.statusLine, isNull);
      },
    );
  });

  group('dispose', () {
    test('closes the port', () {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      final controller = SyncController(port: port);

      controller.dispose();

      expect(port.closeCalls, 1);
    });

    test('closing the port a second time directly stays idempotent', () {
      final port = FakeSyncPort(rootId: 'root-a', rootDir: scratch.path);
      final controller = SyncController(port: port);

      controller.dispose();
      port.close();

      expect(port.closeCalls, 2);
    });
  });
}
