// Widget tests for the picker's one-line sync status (lib/picker/
// picker_view.dart): it appears once the app-open cycle's report is
// unread, truncates to one line, opens the report sheet on tap, and clears
// on close. Also covers the per-row "Sync" menu action a paired root's
// entries gain. Driven through the real PickerScreen with a FakeSyncPort
// injected via PickerScreen.buildSyncPort, so no network and no real
// desktop are needed; RustLib.init() is required for the picker's own
// listing of the real temp root, same as the other picker_screen_*_test.dart
// files.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/bridge/sync_bridge.dart' as sync_bridge;
import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/sync/sync_models.dart'
    show PairedConflictPush, SyncDeckState, SyncEntryState, SyncRenamedEntry;
import 'package:alix_mobile/sync/sync_port.dart';
import 'package:alix_mobile/sync/sync_sheet.dart';
import 'package:alix_mobile/sync_client.dart' show SyncEntries, SyncEntry;

import 'support/deck_fixture.dart';
import 'support/fake_server_client.dart';
import 'support/fake_sync_port.dart';
import 'support/widget_tree_dump.dart';
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

  const config = ServerConfig(
    host: '127.0.0.1',
    port: 7777,
    token: 'abc',
    rootId: 'root-test0000000000000000000000',
  );

  Future<Directory> pairedRoot(Directory support) async {
    await savePairing(config, support: support);
    final dir = Directory(
      sync_bridge.pairedRootDirFor(support: support.path, rootId: 'root-test0000000000000000000000'),
    );
    dir.createSync(recursive: true);
    return dir;
  }

  // The paired desktop's directory (from pairedRoot) is a second, separate
  // listing alongside the phone's own root, never a replacement for it, so
  // every pumpPaired call also mounts the picker on its own fresh, empty
  // phone-own root.
  Future<void> pumpPaired(
    WidgetTester tester, {
    required Directory root,
    required Directory support,
    required SyncPort port,
    Directory? phoneRoot,
    bool Function()? until,
  }) async {
    final ownRoot = phoneRoot ?? tempDir('alix-sync-status-phone-');
    await tester.pumpWidget(
      MaterialApp(
        home: PickerScreen(
          key: UniqueKey(),
          root: ownRoot.path,
          supportDir: support,
          currentThemeId: 'dark',
          onSetTheme: (_) async {},
          buildClient: (_) => FakeServerClient(versionReply: minServerVersion),
          buildSyncPort: (_, _) => port,
        ),
      ),
    );
    await settlePicker(tester, until: until);
  }

  testWidgets(
    'the app-open cycle leaves an unread one-line status that truncates '
    'and opens the report sheet on tap',
    (tester) async {
      final support = tempDir('alix-sync-status-support-');
      final root = await pairedRoot(support);
      final port = FakeSyncPort(rootId: 'root-test0000000000000000000000', rootDir: root.path);
      // A renamed pair makes the cycle non-empty (an empty cycle now leaves
      // no status line at all) without changing the summary text: only
      // landed/conflicts/refused/orphaned feed it.
      port.tidyRenamedImpl = (listed) => const [
        SyncRenamedEntry(old: 'A', new_: 'B'),
      ];

      await pumpPaired(tester, root: root, support: support, port: port);

      expect(find.text('Synced: up to date'), findsOneWidget);
      final statusText = tester.widget<Text>(
        find.descendant(
          of: find.byKey(const Key('sync-status')),
          matching: find.byType(Text),
        ),
      );
      expect(statusText.maxLines, 1);
      expect(statusText.overflow, TextOverflow.ellipsis);
      await expectWidgetTree(
        tester,
        'picker_sync_status_row',
        root: find.byKey(const Key('sync-status')),
      );

      await tester.tap(find.byKey(const Key('sync-status')));
      await tester.pumpAndSettle();
      expect(find.byType(SyncReportSheet), findsOneWidget);
    },
  );

  testWidgets('no pairing means no status line and no per-entry Sync action', (
    tester,
  ) async {
    final root = tempDir('alix-sync-status-unpaired-');
    final support = tempDir('alix-sync-status-unpaired-support-');
    await tester.pumpWidget(
      MaterialApp(
        home: PickerScreen(
          key: UniqueKey(),
          root: root.path,
          supportDir: support,
          currentThemeId: 'dark',
          onSetTheme: (_) async {},
        ),
      ),
    );
    await settlePicker(tester);

    expect(find.byKey(const Key('sync-status')), findsNothing);
  });

  testWidgets(
    "a paired root's entry row menu offers Sync, and tapping it runs a "
    'second cycle scoped to that entry',
    (tester) async {
      final support = tempDir('alix-sync-status-row-support-');
      final root = await pairedRoot(support);
      writeTestDeck('${root.path}/deck.md', '---\ntitle: Deck\n---\n## q\na\n');
      final port = FakeSyncPort(rootId: 'root-test0000000000000000000000', rootDir: root.path);
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-test0000000000000000000000',
        entries: [
          SyncEntry(
            name: 'deck.md',
            kind: 'deck',
            members: 1,
            unpackedBytes: 10,
            digest: 'xxh64-0000000000000002',
            leftOut: [],
          ),
        ],
      );

      await pumpPaired(
        tester,
        root: root,
        support: support,
        port: port,
        until: () => find.byIcon(Icons.more_vert).evaluate().isNotEmpty,
      );
      expect(port.entriesCalls.length, 1);

      await tester.tap(find.byIcon(Icons.more_vert));
      await tester.pumpAndSettle();
      expect(find.text('Sync'), findsOneWidget);
      // A real entries() match now runs a real pull attempt (staging dir,
      // zip target), real dart:io the fake test zone never services on its
      // own; same gotcha as the never-pulled-entry pull below.
      await tester.runAsync(() async {
        await tester.tap(find.text('Sync'));
        final deadline = DateTime.now().add(const Duration(seconds: 2));
        while (port.pullCalls.isEmpty && DateTime.now().isBefore(deadline)) {
          await Future<void>.delayed(const Duration(milliseconds: 5));
        }
        await tester.pumpAndSettle();
      });

      expect(port.entriesCalls.length, 2);
      // The manifest name (deck.md), never the display title (Deck): a
      // titled deck's stem does not carry the .md the entry name needs.
      expect(port.pullCalls, ['deck.md']);
    },
  );

  testWidgets(
    "the phone's own decks and the paired desktop's stay two lists: both "
    'show, the desktop one under its host:port label, and only its rows '
    'offer Sync',
    (tester) async {
      final support = tempDir('alix-sync-two-lists-support-');
      final phoneRoot = tempDir('alix-sync-two-lists-phone-');
      writeTestDeck(
        '${phoneRoot.path}/local.md',
        '---\ntitle: Local Deck\n---\n## q\na\n',
      );
      final root = await pairedRoot(support);
      writeTestDeck(
        '${root.path}/remote.md',
        '---\ntitle: Remote Deck\n---\n## q\na\n',
      );
      final port = FakeSyncPort(rootId: 'root-test0000000000000000000000', rootDir: root.path);

      await pumpPaired(
        tester,
        root: root,
        support: support,
        port: port,
        phoneRoot: phoneRoot,
        until: () => find.text('Remote Deck').evaluate().isNotEmpty,
      );

      expect(find.text('Local Deck'), findsOneWidget);
      expect(find.text('Remote Deck'), findsOneWidget);
      expect(find.text('127.0.0.1:7777'), findsOneWidget);
      // Only the paired desktop's own row carries the overflow menu; a
      // phone-own row never does.
      expect(find.byIcon(Icons.more_vert), findsOneWidget);
    },
  );

  testWidgets(
    "a desktop entry the phone has never pulled shows below the phone's "
    'own entries with its name and size, and tapping it pulls it',
    (tester) async {
      final support = tempDir('alix-sync-available-support-');
      final root = await pairedRoot(support);
      final port = FakeSyncPort(rootId: 'root-test0000000000000000000000', rootDir: root.path);
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-test0000000000000000000000',
        entries: [
          SyncEntry(
            name: 'Biology',
            kind: 'workspace',
            members: 3,
            unpackedBytes: 2048,
            digest: 'xxh64-0000000000000002',
            leftOut: [],
          ),
        ],
      );

      await pumpPaired(tester, root: root, support: support, port: port);

      expect(find.text('Biology'), findsOneWidget);
      expect(find.text('2.0 KB'), findsOneWidget);

      // The pull writes real files (a staging directory, the zip target)
      // before FakeSyncPort.pull is ever reached; the fake test zone never
      // services that dart:io I/O on its own, so this needs runAsync, same
      // as review_screen_sync_push_test.dart's summary push.
      await tester.runAsync(() async {
        await tester.tap(find.text('Biology'));
        final deadline = DateTime.now().add(const Duration(seconds: 2));
        while (port.pullCalls.isEmpty && DateTime.now().isBefore(deadline)) {
          await Future<void>.delayed(const Duration(milliseconds: 5));
        }
        await tester.pumpAndSettle();
      });

      expect(port.pullCalls, ['Biology']);
    },
  );

  testWidgets(
    'a conflicted paired deck opens the conflict choice, never a review '
    'session',
    (tester) async {
      final support = tempDir('alix-sync-conflict-gate-support-');
      final root = await pairedRoot(support);
      writeTestDeck('${root.path}/deck.md', '---\ntitle: Deck\n---\n## q\na\n');
      final port = FakeSyncPort(rootId: 'root-test0000000000000000000000', rootDir: root.path);
      port.pairedEntriesImpl = () => const [
        SyncEntryState(
          entry: 'deck.md',
          kind: 'deck',
          digest: 'xxh64-0000000000000001',
          decks: [
            SyncDeckState(
              deckId: 'deck-1',
              path: 'deck.md',
              unpushed: false,
              conflict: PairedConflictPush(desktopRevision: 2),
            ),
          ],
        ),
      ];

      await pumpPaired(
        tester,
        root: root,
        support: support,
        port: port,
        until: () => find.text('Deck').evaluate().isNotEmpty,
      );

      await tester.tap(find.text('Deck'));
      await tester.pumpAndSettle();

      expect(find.byType(ReviewScreen), findsNothing);
      expect(find.byType(SyncReportSheet), findsOneWidget);
    },
  );

  testWidgets(
    'a pairedEntries failure never crashes the report sheet: it opens with '
    'the sync error and no conflicts',
    (tester) async {
      final support = tempDir('alix-sync-pairedentries-fail-support-');
      final root = await pairedRoot(support);
      final port = FakeSyncPort(rootId: 'root-test0000000000000000000000', rootDir: root.path);
      port.pairedEntriesImpl = () => throw Exception('state file corrupt');

      await pumpPaired(tester, root: root, support: support, port: port);
      expect(tester.takeException(), isNull);

      await tester.tap(find.byKey(const Key('sync-status')));
      await tester.pumpAndSettle();

      expect(find.byType(SyncReportSheet), findsOneWidget);
      expect(
        find.descendant(
          of: find.byType(SyncReportSheet),
          matching: find.textContaining('sync failed'),
        ),
        findsOneWidget,
      );
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets(
    'a startup recovery failure on the paired folder never crashes the '
    'picker: it shows as an unread report instead',
    (tester) async {
      final support = tempDir('alix-sync-recover-fail-support-');
      const config = ServerConfig(
        host: '127.0.0.1',
        port: 7777,
        token: 'abc',
        rootId: 'root-brken000000000000000000000',
      );
      await savePairing(config, support: support);
      final pairedDir = sync_bridge.pairedRootDirFor(
        support: support.path,
        rootId: 'root-brken000000000000000000000',
      );
      // A regular file sits where the paired root must be a directory:
      // pairedRecoverFor's create_dir_all/rollback cannot succeed over it,
      // reproducing a real recovery failure rather than a mocked one.
      Directory(pairedDir).parent.createSync(recursive: true);
      File(pairedDir).writeAsStringSync('not a directory');
      final port = FakeSyncPort(rootId: 'root-brken000000000000000000000', rootDir: pairedDir);

      final phoneRoot = tempDir('alix-sync-recover-fail-phone-');
      await tester.pumpWidget(
        MaterialApp(
          home: PickerScreen(
            key: UniqueKey(),
            root: phoneRoot.path,
            supportDir: support,
            currentThemeId: 'dark',
            onSetTheme: (_) async {},
            buildClient: (_) => FakeServerClient(),
            buildSyncPort: (_, _) => port,
          ),
        ),
      );
      await settlePicker(tester);
      expect(tester.takeException(), isNull);

      await tester.tap(find.byKey(const Key('sync-status')));
      await tester.pumpAndSettle();

      expect(find.byType(SyncReportSheet), findsOneWidget);
      expect(
        find.descendant(
          of: find.byType(SyncReportSheet),
          matching: find.textContaining('could not prepare the paired folder'),
        ),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    're-pairing the active root with a fresh token rebuilds its sync port '
    'without an app restart',
    (tester) async {
      final support = tempDir('alix-sync-repair-support-');
      final phoneRoot = tempDir('alix-sync-repair-phone-');
      const oldConfig = ServerConfig(
        host: '127.0.0.1',
        port: 7777,
        token: 'old-token',
        rootId: 'root-test0000000000000000000000',
      );
      await savePairing(oldConfig, support: support);
      final pairedDir = Directory(
        sync_bridge.pairedRootDirFor(
          support: support.path,
          rootId: 'root-test0000000000000000000000',
        ),
      )..createSync(recursive: true);
      writeTestDeck(
        '${pairedDir.path}/deck.md',
        '---\ntitle: Deck\n---\n## q\na\n',
      );
      final oldPort = FakeSyncPort(
        rootId: 'root-test0000000000000000000000',
        rootDir: pairedDir.path,
      );
      final freshPort = FakeSyncPort(
        rootId: 'root-test0000000000000000000000',
        rootDir: pairedDir.path,
      );
      final builtTokens = <String>[];

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
            buildSyncPort: (config, _) {
              builtTokens.add(config.token);
              return config.token == 'fresh-token' ? freshPort : oldPort;
            },
          ),
        ),
      );
      await settlePicker(tester);
      expect(builtTokens, ['old-token']);

      await tester.tap(find.byIcon(Icons.menu));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Connected devices'));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const ValueKey('pairing-url-field')),
        'http://127.0.0.1:7777/?token=fresh-token',
      );
      await tester.tap(find.text('Pair'));
      await tester.pumpAndSettle();

      expect(readActivePairing(support)?.token, 'fresh-token');
      expect(
        builtTokens,
        ['old-token', 'fresh-token'],
        reason: 'the successful re-pair must stop using the expired client',
      );
    },
  );

  testWidgets(
    'a paired-state read failure while opening a deck is handled instead '
    'of becoming an unhandled widget error',
    (tester) async {
      final support = tempDir('alix-sync-open-state-fail-support-');
      final root = await pairedRoot(support);
      writeTestDeck('${root.path}/deck.md', '---\ntitle: Deck\n---\n## q\na\n');
      final port = FakeSyncPort(rootId: 'root-test0000000000000000000000', rootDir: root.path);
      port.pairedEntriesImpl = () => throw Exception('state file corrupt');

      await pumpPaired(
        tester,
        root: root,
        support: support,
        port: port,
        until: () => find.text('Deck').evaluate().isNotEmpty,
      );
      expect(tester.takeException(), isNull);

      await tester.tap(find.text('Deck'));
      await tester.pump();

      expect(
        tester.takeException(),
        isNull,
        reason: 'conflict gating must not leak a bridge error into InkWell',
      );
      expect(find.byType(ReviewScreen), findsNothing);
    },
  );
}
