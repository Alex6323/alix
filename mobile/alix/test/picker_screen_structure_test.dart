import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/folder_browser.dart';
import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/platform_access.dart';
import 'package:alix_mobile/exam_screen.dart';
import 'package:alix_mobile/exam_sittings.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/settings_screen.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/sync/sync_port.dart';
import 'package:alix_mobile/theme.dart';

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

  Future<void> pumpPicker(
    WidgetTester tester, {
    required Directory root,
    Directory? support,
    PlatformAccess? access,
    Future<void> Function(String?)? onSetTheme,
    ServerClient Function(ServerConfig)? buildClient,
    Duration? pollInterval,
    String? dir,
    String? title,
    SyncPort Function(ServerConfig config, String rootDir)? buildSyncPort,
    bool isPairedSubtree = false,
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
          currentThemeId: 'dark',
          onSetTheme: onSetTheme,
          buildClient: buildClient,
          generatePollInterval: pollInterval,
          buildSyncPort: buildSyncPort,
          isPairedSubtree: isPairedSubtree,
        ),
      ),
    );
    await settlePicker(tester, strips: true);
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
    'picker tree: active root, workspace, mastered, and due markers',
    (tester) async {
      final root = tempDir('alix-picker-structure-active-');
      writeTestDeck(
        '${root.path}/due.md',
        '---\ntitle: Due\n---\n## q\na\n<!-- id: card-due -->\n',
      );
      writeTestDeck(
        '${root.path}/mastered.md',
        '---\ntitle: Mastered\n---\n## q\na\n<!-- id: card-mastered -->\n',
        id: 'mastered',
      );
      Directory('${root.path}/.alix/progress').createSync(recursive: true);
      File('${root.path}/.alix/progress/deck-mastered.json').writeAsStringSync(
        jsonEncode({
          'version': 1,
          'deck_id': 'deck-mastered',
          'subject': 'mastered.md',
          'revision': 1,
          'cards': {},
          'deck': {
            'passed_at_ms': 1,
            'mastered_at_ms': 1,
            'exam_stamp': {'sourced': 'x', 'unsourced_cards': <String>[]},
          },
        }),
      );
      Directory('${root.path}/workspace/decks').createSync(recursive: true);
      File(
        '${root.path}/workspace/alix.toml',
      ).writeAsStringSync('title = "Workspace"\n');
      writeTestDeck(
        '${root.path}/workspace/decks/member.md',
        '---\ntitle: Member\n---\n## q\na\n',
      );

      await pumpPicker(tester, root: root);
      await expectWidgetTree(
        tester,
        'picker_active_root',
        root: find.byType(PickerScreen),
      );

      await tester.tap(find.text('Mastered · 1'));
      await tester.pumpAndSettle();
      await settlePicker(tester, strips: true);
      await expectWidgetTree(
        tester,
        'picker_mastered_window',
        root: find.byType(PickerScreen).last,
      );
    },
  );

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
    File(
      '${workspace.path}/alix.local.toml',
    ).writeAsStringSync('[review]\ndeadline = "2100-01-05"\n');
    writeTestDeck('${decks.path}/base.md', '---\ntitle: Base\n---\n## q?\na\n');
    writeTestSources('${decks.path}/base.md');
    writeTestDeck(
      '${decks.path}/mid.md',
      '---\nrequires: base\ntitle: Mid\n---\n## q?\na\n',
    );
    writeTestDeck(
      '${decks.path}/tip.md',
      '---\nrequires: mid\ntitle: Tip\n---\n## q?\na\n',
    );
    await pumpPicker(tester, root: root, dir: workspace.path, title: 'Path');
    await expectWidgetTree(
      tester,
      'picker_workspace_tree',
      root: find.byType(PickerScreen),
    );
  });

  testWidgets(
    'picker tree: settings, depth, deadline, theme, support, and about sheets',
    (tester) async {
      final root = tempDir('alix-picker-structure-settings-');
      final support = tempDir('alix-picker-structure-settings-support-');
      writeTestDeck('${root.path}/deck.md', '---\ntitle: Deck\n---\n## q\na\n');
      Directory('${root.path}/workspace/decks').createSync(recursive: true);
      File(
        '${root.path}/workspace/alix.toml',
      ).writeAsStringSync('title = "Workspace"\n');
      writeTestDeck(
        '${root.path}/workspace/decks/member.md',
        '---\ntitle: Member\n---\n## q\na\n',
      );
      await pumpPicker(
        tester,
        root: root,
        support: support,
        access: const _FakeAccess(),
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

      await tester.tap(find.text('Deck'));
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
      onSetTheme: (_) async {},
      buildClient: (_) => client,
    );
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
    expect(find.byIcon(Icons.menu), findsOneWidget);
    expect(find.byType(BackButton), findsNothing);
    await tester.tap(find.byIcon(Icons.menu));
    await tester.pumpAndSettle();
    expect(find.text('Generate deck'), findsOneWidget);
    await expectWidgetTree(
      tester,
      'picker_pairing_reprobed',
      root: find.byType(SettingsScreen),
    );
  });

  testWidgets(
    'the exam action follows the remote slot and announces readiness once',
    (tester) async {
      final root = tempDir('alix-picker-exam-ahead-');
      final support = tempDir('alix-picker-exam-ahead-support-');
      Directory('${root.path}/ws/decks').createSync(recursive: true);
      File('${root.path}/ws/alix.toml').writeAsStringSync('');
      final deck = '${root.path}/ws/decks/exam.md';
      writeTestDeck(deck, '---\ntitle: Remote exam\n---\n## q?\na\n');
      writeTestSources(deck);
      await savePairing(
        const ServerConfig(
          host: '127.0.0.1',
          port: 7777,
          token: 'abc',
          rootId: 'root-test0000000000000000000000',
        ),
        support: support,
      );
      const generatingExam = RemoteExam(
        phase: 'generating',
        deck: 'ws/exam.md',
        strictness: 'balanced',
        questions: [],
        grades: [],
        gaps: [],
        canRemediate: false,
        thinking: true,
        estimate: 'about 20 to 30 minutes',
      );
      const answeringExam = RemoteExam(
        phase: 'answering',
        deck: 'ws/exam.md',
        strictness: 'balanced',
        questions: ['Why?'],
        grades: [],
        gaps: [],
        canRemediate: false,
        thinking: false,
      );
      final client = FakeServerClient(
        versionReply: minServerVersion,
        examGetReplies: const [generatingExam, generatingExam, answeringExam],
      );
      await pumpPicker(
        tester,
        root: root,
        dir: '${root.path}/ws',
        support: support,
        buildClient: (_) => client,
        pollInterval: const Duration(seconds: 10),
        buildSyncPort: (_, _) => FakeSyncPort(
          rootId: 'root-test0000000000000000000000',
          rootDir: root.path,
        ),        isPairedSubtree: true,
      );

      await tester.tap(find.text('Remote exam'));
      await tester.pumpAndSettle();
      expect(find.text('Generate exam'), findsOneWidget);
      expect(find.textContaining('preparing…'), findsOneWidget);

      // Polling pauses while another route covers the picker, but it must
      // resume when that route closes: writing the exam is explicitly a
      // background operation the learner may leave to do other work.
      await tester.pump(const Duration(seconds: 10));
      expect(client.examGetCalls, 2);
      await tester.tapAt(const Offset(10, 10));
      await tester.pumpAndSettle();

      await tester.pump(const Duration(seconds: 10));
      await tester.pump();
      expect(find.text('Exam ready: ws/exam.md.'), findsOneWidget);
      final pollsWhenReady = client.examGetCalls;
      await tester.pump(const Duration(seconds: 30));
      expect(
        client.examGetCalls,
        pollsWhenReady,
        reason: 'a ready exam stops the picker polling',
      );

      await tester.tap(find.text('Remote exam'));
      await tester.pumpAndSettle();
      expect(find.text('Take exam'), findsOneWidget);
    },
  );

  Future<({Directory root, Directory support, String deck})> examFixture(
    String prefix,
  ) async {
    final root = tempDir('alix-picker-$prefix-');
    final support = tempDir('alix-picker-$prefix-support-');
    Directory('${root.path}/ws/decks').createSync(recursive: true);
    File('${root.path}/ws/alix.toml').writeAsStringSync('');
    File('${root.path}/ws/decks/source.txt').writeAsStringSync('alpha\nbeta\n');
    final deck = '${root.path}/ws/decks/exam.md';
    writeTestDeck(deck, '---\ntitle: Remote exam\n---\n## q?\na\n');
    writeVerifiedTestSources(deck);
    await savePairing(
      const ServerConfig(
        host: '127.0.0.1',
        port: 7777,
        token: 'abc',
        rootId: 'root-test0000000000000000000000',
      ),
      support: support,
    );
    return (root: root, support: support, deck: deck);
  }

  const failedRemoteExam = RemoteExam(
    sitting: 'sitting-1',
    phase: 'results',
    deck: 'ws/exam.md',
    strictness: 'balanced',
    questions: ['Why?'],
    verdicts: ExamVerdicts(pass: 0, partial: 0, total: 1),
    full: true,
    grades: [],
    gaps: ['the tested idea'],
    canRemediate: true,
    thinking: false,
  );

  Future<void> generateThenTake(WidgetTester tester) async {
    await tester.tap(find.text('Generate exam'));
    await tester.pump();
    await tester.pump(const Duration(seconds: 10));
    await tester.pump();
    expect(
      find.byType(ExamScreen),
      findsNothing,
      reason: 'a finished preparation never opens the exam by itself',
    );
    await tester.tap(find.text('Take exam'));
    await tester.pumpAndSettle();
  }

  Future<void> takeFinishedExam(WidgetTester tester) async {
    await tester.tap(find.text('Remote exam'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Take exam'));
    await tester.pumpAndSettle();
    expect(find.text('0 / 1'), findsOneWidget, reason: 'the finished result opens');
  }

  testWidgets(
    'a reopened sitting is recorded against the stamp taken when it started',
    (tester) async {
      final fixture = await examFixture('exam-reopen');
      const generatingExam = RemoteExam(
        phase: 'generating',
        deck: 'ws/exam.md',
        strictness: 'balanced',
        questions: [],
        grades: [],
        gaps: [],
        canRemediate: false,
        thinking: true,
      );
      final client = FakeServerClient(
        versionReply: minServerVersion,
        noSittingBeforeStart: true,
        examStartSitting: 'sitting-1',
        examGetReplies: const [generatingExam],
      );
      await pumpPicker(
        tester,
        root: fixture.root,
        dir: '${fixture.root.path}/ws',
        support: fixture.support,
        buildClient: (_) => client,
        pollInterval: const Duration(seconds: 10),
        buildSyncPort: (_, _) => FakeSyncPort(
          rootId: 'root-test0000000000000000000000',
          rootDir: fixture.root.path,
        ),
        isPairedSubtree: true,
      );
      await tester.tap(find.text('Remote exam'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Generate exam'));
      for (var i = 0; i < 5; i++) {
        await tester.pump(const Duration(milliseconds: 50));
      }
      expect(client.startedDeck, 'ws/exam.md', reason: 'step 1: the sitting starts');
      expect(
        find.textContaining('preparing…'),
        findsOneWidget,
        reason: 'step 1: the sheet stays open and shows the preparation',
      );

      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      expect(client.examCloseCalls, 0, reason: 'step 2: leaving keeps the sitting');
      client.replaceExam(failedRemoteExam);
      await tester.pump(const Duration(seconds: 10));
      await tester.pump();

      File(fixture.deck).writeAsStringSync(
        '\n# Extra\n\n## q2\na2\n',
        mode: FileMode.append,
      );
      await takeFinishedExam(tester);

      expect(
        find.text('Turn the gaps into cards'),
        findsNothing,
        reason: 'step 3: material added since the sitting started voids it',
      );
      expect(find.textContaining('the exam material changed'), findsOneWidget);
    },
  );

  testWidgets(
    'a preparing exam is cancelled from its row after confirmation',
    (tester) async {
      final fixture = await examFixture('exam-cancel');
      final client = FakeServerClient(
        versionReply: minServerVersion,
        noSittingBeforeStart: true,
        examStartSitting: 'sitting-1',
      );
      await pumpPicker(
        tester,
        root: fixture.root,
        dir: '${fixture.root.path}/ws',
        support: fixture.support,
        buildClient: (_) => client,
        pollInterval: const Duration(seconds: 10),
        buildSyncPort: (_, _) => FakeSyncPort(
          rootId: 'root-test0000000000000000000000',
          rootDir: fixture.root.path,
        ),
        isPairedSubtree: true,
      );
      await tester.tap(find.text('Remote exam'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Generate exam'));
      await tester.pump();
      await tester.pump();
      expect(find.textContaining('preparing…'), findsOneWidget, reason: 'step 1');

      await tester.tap(find.text('Generate exam'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Keep'));
      await tester.pumpAndSettle();
      expect(client.examCloseCalls, 0, reason: 'step 2: Keep keeps it');
      expect(find.textContaining('preparing…'), findsOneWidget, reason: 'step 2');

      // The version probe shares this fake and closed it; the picker's exam
      // client is its own instance in production.
      client.closed = false;
      await tester.tap(find.text('Generate exam'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Cancel preparation'));
      await tester.pumpAndSettle();
      expect(client.examCloseCalls, 1, reason: 'step 3: the desktop is told');
      expect(
        find.text('graded by your paired desktop'),
        findsOneWidget,
        reason: 'step 3: the row offers a fresh generation',
      );
    },
  );

  testWidgets(
    'a sitting started before an app restart is still recorded',
    (tester) async {
      final fixture = await examFixture('exam-restart');
      final before = FakeServerClient(
        versionReply: minServerVersion,
        noSittingBeforeStart: true,
        examStartSitting: 'sitting-1',
      );
      Future<void> pump(FakeServerClient client) => pumpPicker(
        tester,
        root: fixture.root,
        dir: '${fixture.root.path}/ws',
        support: fixture.support,
        buildClient: (_) => client,
        pollInterval: const Duration(seconds: 10),
        buildSyncPort: (_, _) => FakeSyncPort(
          rootId: 'root-test0000000000000000000000',
          rootDir: fixture.root.path,
        ),
        isPairedSubtree: true,
      );
      await pump(before);
      await tester.tap(find.text('Remote exam'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Generate exam'));
      await tester.pump();
      await tester.pump();
      expect(before.startedDeck, 'ws/exam.md', reason: 'step 1: started');

      await tester.pumpWidget(const SizedBox());
      StartedSittings.forgetLoaded();
      final after = FakeServerClient(
        versionReply: minServerVersion,
        examGetReplies: const [failedRemoteExam],
      );
      await pump(after);
      await takeFinishedExam(tester);

      expect(
        find.textContaining('not recorded'),
        findsNothing,
        reason: 'step 2: the restarted app still knows it started this sitting',
      );
      expect(find.text('Turn the gaps into cards'), findsOneWidget);
    },
  );

  testWidgets(
    'a finished sitting this phone did not start is not recorded and offers no remediation',
    (tester) async {
      final fixture = await examFixture('exam-unstarted');
      final client = FakeServerClient(
        versionReply: minServerVersion,
        examGetReplies: const [failedRemoteExam],
      );
      await pumpPicker(
        tester,
        root: fixture.root,
        dir: '${fixture.root.path}/ws',
        support: fixture.support,
        buildClient: (_) => client,
        pollInterval: const Duration(seconds: 10),
        buildSyncPort: (_, _) => FakeSyncPort(
          rootId: 'root-test0000000000000000000000',
          rootDir: fixture.root.path,
        ),
        isPairedSubtree: true,
      );
      await takeFinishedExam(tester);

      expect(find.text('Turn the gaps into cards'), findsNothing);
      expect(
        find.textContaining('not recorded'),
        findsOneWidget,
        reason: 'the phone says the result was not recorded',
      );
    },
  );

  testWidgets(
    'a remediated sitting this phone did not start writes no personal cards',
    (tester) async {
      final fixture = await examFixture('exam-unstarted-remediated');
      const remediatedExam = RemoteExam(
        sitting: 'sitting-from-another-client',
        phase: 'remediated',
        deck: 'ws/exam.md',
        strictness: 'balanced',
        questions: ['Why?'],
        grades: [],
        gaps: ['the tested idea'],
        canRemediate: false,
        cardsByTopic: [
          RemoteTopicCards(
            topicId: 'topic-remediation',
            cards: '## Why?\nBecause.\n',
          ),
        ],
        thinking: false,
      );
      final client = FakeServerClient(
        versionReply: minServerVersion,
        examGetReplies: const [remediatedExam],
      );
      await pumpPicker(
        tester,
        root: fixture.root,
        dir: '${fixture.root.path}/ws',
        support: fixture.support,
        buildClient: (_) => client,
        pollInterval: const Duration(seconds: 10),
        buildSyncPort: (_, _) => FakeSyncPort(
          rootId: 'root-test0000000000000000000000',
          rootDir: fixture.root.path,
        ),
        isPairedSubtree: true,
      );

      await tester.tap(find.text('Remote exam'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Take exam'));
      await tester.pumpAndSettle();

      expect(find.text('Done.'), findsOneWidget);
      expect(
        find.text('0 new cards to drill.'),
        findsNothing,
        reason:
            'a remote sitting this phone refuses to apply must not report a local remediation result',
      );
      expect(
        fixture.root
            .listSync(recursive: true)
            .whereType<File>()
            .where((file) => file.path.endsWith('.local.md')),
        isEmpty,
        reason:
            'another client sitting is display-only on this phone and must not write remediation cards',
      );
      expect(
        fixture.root
            .listSync(recursive: true)
            .whereType<File>()
            .where((file) => file.path.contains('/.alix/progress/')),
        isEmpty,
        reason: 'the refused sitting must not create phone progress either',
      );
    },
  );

  testWidgets(
    'a replacement sitting on the same deck does not reuse this phone previous start',
    (tester) async {
      final fixture = await examFixture('exam-replaced');
      const generatingExam = RemoteExam(
        phase: 'generating',
        deck: 'ws/exam.md',
        strictness: 'balanced',
        questions: [],
        grades: [],
        gaps: [],
        canRemediate: false,
        thinking: true,
      );
      final client = FakeServerClient(
        versionReply: minServerVersion,
        noSittingBeforeStart: true,
        examStartSitting: 'sitting-1',
        examGetReplies: const [generatingExam],
      );
      await pumpPicker(
        tester,
        root: fixture.root,
        dir: '${fixture.root.path}/ws',
        support: fixture.support,
        buildClient: (_) => client,
        pollInterval: const Duration(seconds: 10),
        buildSyncPort: (_, _) => FakeSyncPort(
          rootId: 'root-test0000000000000000000000',
          rootDir: fixture.root.path,
        ),
        isPairedSubtree: true,
      );
      await tester.tap(find.text('Remote exam'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Generate exam'));
      for (var i = 0; i < 5; i++) {
        await tester.pump(const Duration(milliseconds: 50));
      }
      expect(
        client.startedDeck,
        'ws/exam.md',
        reason: 'step 1: this phone starts a sitting',
      );

      client.replaceExam(const RemoteExam(
        sitting: 'sitting-2',
        phase: 'results',
        deck: 'ws/exam.md',
        strictness: 'balanced',
        questions: ['Why?'],
        verdicts: ExamVerdicts(pass: 0, partial: 0, total: 1),
        full: true,
        grades: [],
        gaps: ['the tested idea'],
        canRemediate: true,
        thinking: false,
      ));
      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      await tester.pump(const Duration(seconds: 10));
      await tester.pump();
      await takeFinishedExam(tester);

      expect(
        find.text('Turn the gaps into cards'),
        findsNothing,
        reason:
            'step 2: another client sitting must not write this phone store',
      );
      expect(find.textContaining('not recorded'), findsOneWidget);
    },
  );

  testWidgets(
    'reopening after close failure does not record one result twice',
    (tester) async {
      final fixture = await examFixture('exam-close-failure');
      final client = FakeServerClient(
        versionReply: minServerVersion,
        noSittingBeforeStart: true,
        examStartSitting: 'sitting-1',
        examGetReplies: const [failedRemoteExam],
        failExamClose: true,
      );
      await pumpPicker(
        tester,
        root: fixture.root,
        dir: '${fixture.root.path}/ws',
        support: fixture.support,
        buildClient: (_) => client,
        pollInterval: const Duration(seconds: 10),
        buildSyncPort: (_, _) => FakeSyncPort(
          rootId: 'root-test0000000000000000000000',
          rootDir: fixture.root.path,
        ),
        isPairedSubtree: true,
      );
      await tester.tap(find.text('Remote exam'));
      await tester.pumpAndSettle();
      await generateThenTake(tester);
      expect(
        find.text('0 / 1'),
        findsOneWidget,
        reason: 'step 1: the result opens',
      );
      final progress = fixture.root
          .listSync(recursive: true)
          .whereType<File>()
          .singleWhere((file) => file.path.contains('/.alix/progress/'));
      final first =
          jsonDecode(progress.readAsStringSync()) as Map<String, dynamic>;

      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      expect(client.examCloseCalls, 1, reason: 'step 2: closing failed once');
      await takeFinishedExam(tester);
      final reopened =
          jsonDecode(progress.readAsStringSync()) as Map<String, dynamic>;

      expect(
        reopened['revision'],
        first['revision'],
        reason: 'step 3: reopening the retained result must not save it again',
      );
      expect(
        find.text('Turn the gaps into cards'),
        findsOneWidget,
        reason: 'step 3: the recorded result still offers remediation',
      );
    },
  );

  testWidgets(
    'reopening a remediated sitting after close failure does not apply it twice',
    (tester) async {
      final fixture = await examFixture('exam-remediation-close-failure');
      const remediatedExam = RemoteExam(
        sitting: 'sitting-1',
        phase: 'remediated',
        deck: 'ws/exam.md',
        strictness: 'balanced',
        questions: ['Why?'],
        grades: [],
        gaps: ['the tested idea'],
        canRemediate: false,
        cardsByTopic: [
          RemoteTopicCards(
            topicId: 'topic-remediation',
            cards: '## Why?\nBecause.\n',
          ),
        ],
        thinking: false,
      );
      final client = FakeServerClient(
        versionReply: minServerVersion,
        noSittingBeforeStart: true,
        examStartSitting: 'sitting-1',
        examGetReplies: const [failedRemoteExam],
        examGetRepliesAfterRemediate: const [remediatedExam],
        failExamClose: true,
      );
      await pumpPicker(
        tester,
        root: fixture.root,
        dir: '${fixture.root.path}/ws',
        support: fixture.support,
        buildClient: (_) => client,
        pollInterval: const Duration(seconds: 10),
        buildSyncPort: (_, _) => FakeSyncPort(
          rootId: 'root-test0000000000000000000000',
          rootDir: fixture.root.path,
        ),
        isPairedSubtree: true,
      );
      await tester.tap(find.text('Remote exam'));
      await tester.pumpAndSettle();
      await generateThenTake(tester);
      await tester.tap(find.text('Turn the gaps into cards'));
      await tester.pump(const Duration(seconds: 10));
      await tester.pumpAndSettle();
      expect(
        find.text('Done.'),
        findsOneWidget,
        reason: 'step 1: remediation lands',
      );
      expect(
        find.text('1 new cards to drill.'),
        findsOneWidget,
        reason: 'step 1: the first remediation creates one card',
      );
      final progress = fixture.root
          .listSync(recursive: true)
          .whereType<File>()
          .singleWhere((file) => file.path.contains('/.alix/progress/'));
      final first =
          jsonDecode(progress.readAsStringSync()) as Map<String, dynamic>;

      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      expect(client.examCloseCalls, 1, reason: 'step 2: closing failed once');
      await tester.pump(const Duration(seconds: 5));
      await tester.pumpAndSettle();
      expect(
        find.text('1 new cards to drill.'),
        findsNothing,
        reason: 'step 2: the first remediation snack is gone',
      );
      await tester.tap(find.text('Remote exam'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Take exam'));
      await tester.pumpAndSettle();
      final reopened =
          jsonDecode(progress.readAsStringSync()) as Map<String, dynamic>;

      expect(
        reopened['revision'],
        first['revision'],
        reason: 'step 3: reopening retained remediation must not save it again',
      );
      expect(
        find.text('1 new cards to drill.'),
        findsOneWidget,
        reason: 'step 4: the reopen reports the remediation it kept, not a rerun',
      );
    },
  );

  testWidgets(
    'picker tree: generate unavailable, idle, busy, failed, and destination states',
    (tester) async {
      final root = tempDir('alix-picker-structure-generate-');
      final support = tempDir('alix-picker-structure-generate-support-');
      await savePairing(
        const ServerConfig(
          host: '127.0.0.1',
          port: 7777,
          token: 'abc',
          rootId: 'root-test0000000000000000000000',
        ),
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
        onSetTheme: (_) async {},
        buildClient: (_) => busyClient,
        pollInterval: const Duration(seconds: 10),
        // A pairing is active (Settings needs it to offer Generate), so the
        // root screen now also runs the app-open sync cycle; a fake port
        // keeps it off the real network and, listing nothing, leaves no
        // status line for this test's tree to capture.
        buildSyncPort: (_, _) => FakeSyncPort(
          rootId: 'root-test0000000000000000000000',
          rootDir: root.path,
        ),
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
}
