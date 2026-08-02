import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/folder_browser.dart';
import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/platform_access.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/settings_screen.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

import 'support/deck_fixture.dart';
import 'support/fake_server_client.dart';
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

  Future<void> pumpPicker(
    WidgetTester tester, {
    required Directory root,
    Directory? support,
    PlatformAccess? access,
    Future<void> Function(String?)? onSetDecksDir,
    Future<void> Function(String?)? onSetTheme,
    ServerClient Function(ServerConfig)? buildClient,
    Duration? pollInterval,
    String? dir,
    String? title,
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: PickerScreen(
          key: UniqueKey(),
          root: root.path,
          dir: dir,
          title: title,
          supportDir: support,
          access: access,
          onSetDecksDir: onSetDecksDir,
          currentThemeId: 'dark',
          onSetTheme: onSetTheme,
          buildClient: buildClient,
          generatePollInterval: pollInterval,
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> openSettings(WidgetTester tester) async {
    await tester.tap(find.byIcon(Icons.menu));
    await tester.pumpAndSettle();
  }

  testWidgets('picker tree: empty root', (tester) async {
    final root = tempDir('alix-picker-structure-empty-');
    await pumpPicker(tester, root: root);
    await expectWidgetTree(
      tester,
      'picker_empty_root',
      root: find.byType(PickerScreen),
    );
  });

  testWidgets(
    'picker tree: active root, workspace, mastered, due, and exam markers',
    (tester) async {
      final root = tempDir('alix-picker-structure-active-');
      writeTestDeck(
        '${root.path}/due.md',
        '# Due\n\n## q <!-- id: card-due -->\na\n',
      );
      final exam = '${root.path}/exam.md';
      writeTestDeck(
        exam,
        '---\nsource: https://example.com\n---\n# Exam\n\n## q?\na\n',
      );
      final now = DateTime.now().millisecondsSinceEpoch;
      final t0 = now - 902000;
      ReviewSession.open(
        deckPath: exam,
        rootDir: root.path,
        nowMs: BigInt.from(t0),
      ).acquire(nowMs: BigInt.from(t0));
      ReviewSession.open(
        deckPath: exam,
        rootDir: root.path,
        nowMs: BigInt.from(t0 + 301000),
      ).grade(grade: Grade.pass, nowMs: BigInt.from(t0 + 301000));
      ReviewSession.open(
        deckPath: exam,
        rootDir: root.path,
        nowMs: BigInt.from(t0 + 902000),
      ).grade(grade: Grade.pass, nowMs: BigInt.from(t0 + 902000));

      writeTestDeck(
        '${root.path}/mastered.md',
        '# Mastered\n\n## q <!-- id: card-mastered -->\na\n',
        id: 'mastered',
      );
      Directory('${root.path}/progress').createSync();
      File('${root.path}/progress/deck-mastered.json').writeAsStringSync(
        jsonEncode({
          'version': 1,
          'deck_id': 'deck-mastered',
          'subject': 'mastered.md',
          'revision': 1,
          'cards': {},
          'deck': {'mastered_at_ms': 1},
        }),
      );
      Directory('${root.path}/workspace/decks').createSync(recursive: true);
      File(
        '${root.path}/workspace/alix.toml',
      ).writeAsStringSync('title = "Workspace"\n');
      writeTestDeck(
        '${root.path}/workspace/decks/member.md',
        '# Member\n\n## q\na\n',
      );

      await pumpPicker(tester, root: root);
      await expectWidgetTree(
        tester,
        'picker_active_root',
        root: find.byType(PickerScreen),
      );

      await tester.tap(find.text('Mastered · 1'));
      await tester.pumpAndSettle();
      await expectWidgetTree(
        tester,
        'picker_mastered_window',
        root: find.byType(PickerScreen).last,
      );
    },
  );

  testWidgets('picker tree: conflict banner before and after dismissal', (
    tester,
  ) async {
    final root = tempDir('alix-picker-structure-conflict-');
    writeTestDeck('${root.path}/deck.md', '# Deck\n\n## q\na\n');
    Directory('${root.path}/progress').createSync();
    File(
      '${root.path}/progress/deck.sync-conflict-20260801.json',
    ).writeAsStringSync('{}');
    await pumpPicker(tester, root: root);
    await expectWidgetTree(
      tester,
      'picker_conflict_visible',
      root: find.byType(PickerScreen),
    );
    await tester.tap(find.byIcon(Icons.close));
    await tester.pump();
    await expectWidgetTree(
      tester,
      'picker_conflict_dismissed',
      root: find.byType(PickerScreen),
    );
  });

  testWidgets('picker tree: workspace dependency tree, locks, and deadline', (
    tester,
  ) async {
    final root = tempDir('alix-picker-structure-workspace-');
    final workspace = Directory('${root.path}/workspace')..createSync();
    final decks = Directory('${workspace.path}/decks')..createSync();
    File('${workspace.path}/alix.toml').writeAsStringSync('title = "Path"\n');
    // A fixed far-future deadline: a computed now-relative date bakes the
    // capture day's rendering into the committed baseline (it went red the
    // day after recording).
    File('${workspace.path}/alix.local.toml').writeAsStringSync(
      '[review]\ndeadline = "2100-01-05"\n',
    );
    writeTestDeck(
      '${decks.path}/base.md',
      '---\nsource: https://example.com\n---\n# Base\n\n## q?\na\n',
    );
    writeTestDeck(
      '${decks.path}/mid.md',
      '---\nrequires: base\n---\n# Mid\n\n## q?\na\n',
    );
    writeTestDeck(
      '${decks.path}/tip.md',
      '---\nrequires: mid\n---\n# Tip\n\n## q?\na\n',
    );
    await pumpPicker(tester, root: root, dir: workspace.path, title: 'Path');
    await expectWidgetTree(
      tester,
      'picker_workspace_tree',
      root: find.byType(PickerScreen),
    );
  });

  testWidgets(
    'picker tree: settings, depth, deadline, folder, theme, support, and about sheets',
    (tester) async {
      final root = tempDir('alix-picker-structure-settings-');
      final support = tempDir('alix-picker-structure-settings-support-');
      writeTestDeck('${root.path}/deck.md', '# Deck\n\n## q\na\n');
      Directory('${root.path}/workspace/decks').createSync(recursive: true);
      File(
        '${root.path}/workspace/alix.toml',
      ).writeAsStringSync('title = "Workspace"\n');
      writeTestDeck(
        '${root.path}/workspace/decks/member.md',
        '# Member\n\n## q\na\n',
      );
      await pumpPicker(
        tester,
        root: root,
        support: support,
        access: const _FakeAccess(),
        onSetDecksDir: (_) async {},
        onSetTheme: (_) async {},
        buildClient: (_) => FakeServerClient(),
      );

      await openSettings(tester);
      await expectWidgetTree(
        tester,
        'picker_settings',
        root: find.byType(SettingsScreen),
      );
      await tester.tap(find.text('Support alix'));
      await tester.pumpAndSettle();
      await expectWidgetTree(
        tester,
        'picker_support_sheet',
        root: find.byType(MaterialApp),
      );
      await tester.tapAt(const Offset(10, 10));
      await tester.pumpAndSettle();

      await tester.tap(find.text('Theme'));
      await tester.pumpAndSettle();
      await expectWidgetTree(
        tester,
        'picker_theme_sheet',
        root: find.byType(MaterialApp),
      );
      await tester.tapAt(const Offset(10, 10));
      await tester.pumpAndSettle();

      await tester.tap(find.text('About'));
      await tester.pumpAndSettle();
      await expectWidgetTree(
        tester,
        'picker_about_dialog',
        root: find.byType(MaterialApp),
      );
      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();
      await tester.tap(find.byType(BackButton));
      await tester.pumpAndSettle();

      await tester.longPress(find.text('Deck'));
      await tester.pumpAndSettle();
      await expectWidgetTree(
        tester,
        'picker_depth_sheet',
        root: find.byType(MaterialApp),
      );
      await tester.tapAt(const Offset(10, 10));
      await tester.pumpAndSettle();

      await tester.longPress(find.text('Workspace'));
      await tester.pumpAndSettle();
      await expectWidgetTree(
        tester,
        'picker_deadline_sheet',
        root: find.byType(MaterialApp),
      );
      await tester.tapAt(const Offset(10, 10));
      await tester.pumpAndSettle();

      await openSettings(tester);
      await tester.tap(find.text('Decks folder'));
      await tester.pumpAndSettle();
      await expectWidgetTree(
        tester,
        'picker_folder_sheet',
        root: find.byType(MaterialApp),
      );
    },
  );

  testWidgets('picker tree: pairing entry affordance and post-close re-probe', (
    tester,
  ) async {
    final root = tempDir('alix-picker-structure-pairing-');
    final support = tempDir('alix-picker-structure-pairing-support-');
    final client = FakeServerClient(versionReply: minServerVersion);
    await pumpPicker(
      tester,
      root: root,
      support: support,
      access: const _FakeAccess(),
      onSetDecksDir: (_) async {},
      onSetTheme: (_) async {},
      buildClient: (_) => client,
    );
    final reopenSettings = tester
        .widget<IconButton>(find.widgetWithIcon(IconButton, Icons.menu))
        .onPressed!;
    await openSettings(tester);
    expect(find.text('Connected devices'), findsOneWidget);
    expect(find.text('Generate deck'), findsNothing);
    await expectWidgetTree(
      tester,
      'picker_pairing_entry',
      root: find.byType(SettingsScreen),
    );

    await tester.tap(find.text('Connected devices'));
    await tester.pumpAndSettle();
    await tester.enterText(
      find.byKey(const ValueKey('pairing-url-field')),
      'http://127.0.0.1:7777/?token=abc',
    );
    await tester.tap(find.text('Pair'));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('pairing-url-field')), findsNothing);
    expect(find.byType(SettingsScreen), findsOneWidget);
    await tester.pageBack();
    await tester.pumpAndSettle();
    expect(find.byType(PickerScreen), findsOneWidget);
    // Keep this characterization on the pairing liveness re-probe. The
    // route's menu callback remains valid even if the outgoing Settings
    // route caused the picker's app bar to build once with a back arrow.
    reopenSettings();
    await tester.pumpAndSettle();
    expect(find.text('Generate deck'), findsOneWidget);
    await expectWidgetTree(
      tester,
      'picker_pairing_reprobed',
      root: find.byType(SettingsScreen),
    );
  });

  testWidgets(
    'picker tree: generate unavailable, idle, busy, failed, and destination states',
    (tester) async {
      final root = tempDir('alix-picker-structure-generate-');
      final support = tempDir('alix-picker-structure-generate-support-');
      await setServer(
        const ServerConfig(host: '127.0.0.1', port: 7777, token: 'abc'),
        support: support,
      );
      final busyClient = FakeServerClient(
        versionReply: minServerVersion,
        generateGetReplies: const [
          RemoteGenerate(phase: 'generating', elapsed: 2),
          RemoteGenerate(
            phase: 'done',
            deck: '## q\na\n',
            filename: 'generated.md',
            cards: 1,
          ),
        ],
      );
      await pumpPicker(
        tester,
        root: root,
        support: support,
        access: const _FakeAccess(),
        onSetDecksDir: (_) async {},
        onSetTheme: (_) async {},
        buildClient: (_) => busyClient,
        pollInterval: const Duration(seconds: 10),
      );
      await openSettings(tester);
      await tester.tap(find.text('Generate deck'));
      await tester.pumpAndSettle();
      await expectWidgetTree(
        tester,
        'picker_generate_idle',
        root: find.byType(MaterialApp),
      );
      await tester.enterText(
        find.byKey(const ValueKey('generate-url-field')),
        'file:///local',
      );
      await tester.tap(find.text('Generate'));
      await tester.pump();
      await expectWidgetTree(
        tester,
        'picker_generate_failed',
        root: find.byType(MaterialApp),
      );
      await tester.enterText(
        find.byKey(const ValueKey('generate-url-field')),
        'https://example.com',
      );
      await tester.tap(find.text('Generate'));
      await tester.pump();
      await tester.pump();
      await tester.pump();
      await expectWidgetTree(
        tester,
        'picker_generate_busy',
        root: find.byType(MaterialApp),
      );
      await tester.pump(const Duration(seconds: 10));
      await tester.pumpAndSettle();
      expect(find.byType(FolderBrowser), findsOneWidget);
      await expectWidgetTree(
        tester,
        'picker_generate_destination',
        root: find.byType(FolderBrowser),
      );
    },
  );
}

class _FakeAccess implements PlatformAccess {
  const _FakeAccess();

  @override
  Future<String?> appVersion() async => '0.2.0+3';

  @override
  Future<bool> ensureAllFilesAccess() async => true;

  @override
  Future<bool> hasAllFilesAccess() async => true;

  @override
  Future<String?> pickDirectory() async => null;

  @override
  Future<bool> supportsSharedFolders() async => true;
}
