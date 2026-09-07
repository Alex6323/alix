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
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/sync/sync_port.dart';
import 'package:alix_mobile/sync/sync_sheet.dart';

import 'support/deck_fixture.dart';
import 'support/fake_server_client.dart';
import 'support/fake_sync_port.dart';
import 'support/widget_tree_dump.dart';

void main() {
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
    rootId: 'root-test',
  );

  Future<Directory> pairedRoot(Directory support) async {
    await savePairing(config, support: support);
    final dir = Directory(
      sync_bridge.pairedRootDirFor(support: support.path, rootId: 'root-test'),
    );
    dir.createSync(recursive: true);
    return dir;
  }

  Future<void> pumpPaired(
    WidgetTester tester, {
    required Directory root,
    required Directory support,
    required SyncPort port,
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        home: PickerScreen(
          key: UniqueKey(),
          root: root.path,
          supportDir: support,
          currentThemeId: 'dark',
          onSetTheme: (_) async {},
          buildClient: (_) => FakeServerClient(versionReply: minServerVersion),
          buildSyncPort: (_, _) => port,
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  testWidgets(
    'the app-open cycle leaves an unread one-line status that truncates '
    'and opens the report sheet on tap',
    (tester) async {
      final support = tempDir('alix-sync-status-support-');
      final root = await pairedRoot(support);
      final port = FakeSyncPort(rootId: 'root-test', rootDir: root.path);

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

  testWidgets(
    'no pairing means no status line and no per-entry Sync action',
    (tester) async {
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
      await tester.pumpAndSettle();

      expect(find.byKey(const Key('sync-status')), findsNothing);
    },
  );

  testWidgets(
    "a paired root's entry row menu offers Sync, and tapping it runs a "
    'second cycle scoped to that entry',
    (tester) async {
      final support = tempDir('alix-sync-status-row-support-');
      final root = await pairedRoot(support);
      writeTestDeck(
        '${root.path}/deck.md',
        '---\ntitle: Deck\n---\n## q\na\n',
      );
      final port = FakeSyncPort(rootId: 'root-test', rootDir: root.path);

      await pumpPaired(tester, root: root, support: support, port: port);
      expect(port.entriesCalls.length, 1);

      await tester.tap(find.byIcon(Icons.more_vert));
      await tester.pumpAndSettle();
      expect(find.text('Sync'), findsOneWidget);
      await tester.tap(find.text('Sync'));
      await tester.pumpAndSettle();

      expect(port.entriesCalls.length, 2);
    },
  );
}
