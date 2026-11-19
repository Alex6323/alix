import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/bridge/exam_bridge.dart' as exam_bridge;
import 'package:alix_mobile/bridge/sync_bridge.dart' as sync_bridge;
import 'package:alix_mobile/folder_browser.dart';
import 'package:alix_mobile/picker/picker_widgets.dart';
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
      await tester.tap(find.text('Support Alix'));
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
    expect(
      find.text('Connected devices'),
      findsOneWidget,
      reason: 'settings before pairing: one Connected devices entry',
    );
    expect(
      find.text('Generate deck'),
      findsNothing,
      reason: 'settings before pairing: no Generate deck entry',
    );
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
    expect(
      find.byKey(const ValueKey('pairing-url-field')),
      findsNothing,
      reason: 'after Pair: the pairing field is closed',
    );
    expect(
      find.byType(SettingsScreen),
      findsOneWidget,
      reason: 'after Pair: one settings screen is back',
    );
    await tester.pageBack();
    await tester.pumpAndSettle();
    expect(
      find.byType(PickerScreen),
      findsOneWidget,
      reason: 'back from settings: one picker screen',
    );
    expect(
      find.byIcon(Icons.menu),
      findsOneWidget,
      reason: 'back from settings: the root picker shows one menu button',
    );
    expect(
      find.byType(BackButton),
      findsNothing,
      reason: 'back from settings: the root picker shows no back button',
    );
    await tester.tap(find.byIcon(Icons.menu));
    await tester.pumpAndSettle();
    expect(
      find.text('Generate deck'),
      findsOneWidget,
      reason: 'settings after pairing: one Generate deck entry',
    );
    await expectWidgetTree(
      tester,
      'picker_pairing_reprobed',
      root: find.byType(SettingsScreen),
    );
  });

  String contract(String name) =>
      File('../../tests/contracts/$name.json').readAsStringSync();
  String corpusSitting() =>
      ((jsonDecode(contract('CarriedReplyDto')) as Map)['exam'] as Map)['sitting'] as String;

  const pairedRootId = 'root-test0000000000000000000000';
  const otherRootId = 'root-zzzz0000000000000000000000';
  const idleDesk = RemoteExam(phase: 'idle', deck: '', thinking: false);
  RemoteExam deskState(String phase, String? sitting) => RemoteExam(
    sitting: sitting,
    phase: phase,
    deck: 'ws/exam.md',
    thinking: phase == 'generating',
  );

  ServerConfig pairing(String rootId) =>
      ServerConfig(host: '127.0.0.1', port: 7777, token: 'abc', rootId: rootId);

  Directory pairedWorkspace(Directory support, String rootId) {
    final root = Directory(
      sync_bridge.pairedRootDirFor(support: support.path, rootId: rootId),
    );
    Directory('${root.path}/ws/decks').createSync(recursive: true);
    File('${root.path}/ws/alix.toml').writeAsStringSync('');
    File('${root.path}/ws/decks/source.txt').writeAsStringSync('alpha\nbeta\n');
    return root;
  }

  String writeExamDeck(Directory root, String file, String title) {
    final deck = '${root.path}/ws/decks/$file';
    writeTestDeck(deck, '---\ntitle: $title\n---\n## q?\na\n');
    writeVerifiedTestSources(deck);
    return deck;
  }

  Future<({Directory root, Directory support, String deck})> examFixture(
    String prefix,
  ) async {
    final support = tempDir('alix-picker-$prefix-support-');
    await savePairing(pairing(pairedRootId), support: support);
    final root = pairedWorkspace(support, pairedRootId);
    final deck = writeExamDeck(root, 'exam.md', 'Remote exam');
    return (root: root, support: support, deck: deck);
  }

  FakeServerClient fetchingDesk({
    List<String?>? carried,
    Completer<void>? examCloseGate,
    bool examCloseReply = true,
    Completer<void>? gradingCloseGate,
    bool expireOnGradingClose = false,
  }) => FakeServerClient(
    versionReply: minServerVersion,
    idleBeforeStart: true,
    examStartSitting: corpusSitting(),
    examGetReplies: [deskState('ready', corpusSitting())],
    examCarriedReply: contract('CarriedReplyDto'),
    examCarriedReplies: carried,
    examCloseGate: examCloseGate,
    examCloseReply: examCloseReply,
    gradingCloseGate: gradingCloseGate,
    expireOnGradingClose: expireOnGradingClose,
  );

  Future<List<_DeskHandle>> pumpExamPicker(
    WidgetTester tester, {
    required Directory root,
    required Directory support,
    required FakeServerClient Function(ServerConfig) deskFor,
  }) async {
    final handles = <_DeskHandle>[];
    await pumpPicker(
      tester,
      root: root,
      dir: '${root.path}/ws',
      support: support,
      buildClient: (config) {
        final handle = _DeskHandle(deskFor(config));
        handles.add(handle);
        return handle;
      },
      pollInterval: const Duration(seconds: 10),
      buildSyncPort: (_, _) =>
          FakeSyncPort(rootId: pairedRootId, rootDir: root.path),
      isPairedSubtree: true,
    );
    return handles;
  }

  Future<void> openDeck(WidgetTester tester, [String title = 'Remote exam']) async {
    await tester.tap(find.text(title));
    await tester.pumpAndSettle();
  }

  Future<void> closeSheet(WidgetTester tester) async {
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
  }

  Future<void> tick(WidgetTester tester) async {
    await tester.pump(const Duration(seconds: 10));
    await tester.pump();
  }

  Future<void> tapGenerate(WidgetTester tester) async {
    await tester.tap(find.text('Generate exam'));
    await tester.pump();
    await tester.pump();
  }

  Future<void> generateAndFetch(WidgetTester tester) async {
    await openDeck(tester);
    await tapGenerate(tester);
    await closeSheet(tester);
    await tick(tester);
  }

  List<File> heldFiles(Directory root) {
    final folder = Directory('${root.path}/.alix/exam');
    if (!folder.existsSync()) return const [];
    return folder.listSync().whereType<File>().toList();
  }

  List<Map<String, dynamic>> records(Directory support) {
    final file = File('${support.path}/exam-sittings.json');
    if (!file.existsSync()) return const [];
    final json = jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
    return [for (final record in json['sittings'] as List) record as Map<String, dynamic>];
  }

  void forgetFetched(Directory support) {
    final file = File('${support.path}/exam-sittings.json');
    file.writeAsStringSync(
      file.readAsStringSync().replaceAll('"fetched":true', '"fetched":false'),
    );
    StartedSittings.forgetLoaded();
  }

  void storeResult(String deck, Directory root) {
    exam_bridge.BridgeHeldExam(
      deckPath: deck,
      deckId: exam_bridge.examDeckId(deckPath: deck)!,
      rootDir: root.path,
    ).storeResult(contract('GradingReply.results'), recorded: false);
  }

  void blockRecordSaves(Directory support) =>
      Directory('${support.path}/exam-sittings.json.tmp').createSync();

  Future<void> chooseDiscard(WidgetTester tester) async {
    await tester.tap(find.byIcon(Icons.more_vert));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Discard exam'));
    await tester.pumpAndSettle();
  }

  Future<void> confirmDiscard(WidgetTester tester) async {
    await chooseDiscard(tester);
    await tester.tap(
      find.descendant(of: find.byType(AlertDialog), matching: find.text('Discard exam')),
    );
    await tester.pumpAndSettle();
  }

  Future<void> leaveExam(WidgetTester tester) async {
    await tester.tap(find.byIcon(Icons.close));
    await tester.pumpAndSettle();
  }

  testWidgets(
    'the picker polls every interval while its own exam is ready and unfetched, and stops once fetched',
    (tester) async {
      final fixture = await examFixture('exam-ready-unfetched');
      final desk = fetchingDesk(carried: [null, contract('CarriedReplyDto')]);
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await openDeck(tester);
      await tapGenerate(tester);
      await closeSheet(tester);

      await tick(tester);
      expect(
        [desk.carriedSittings.length, heldFiles(fixture.root).length],
        [1, 0],
        reason: 'first ready poll: one fetch that got nothing, so no held file',
      );
      final pollsWhileUnfetched = desk.examGetCalls;
      await tick(tester);
      expect(
        [desk.examGetCalls, heldFiles(fixture.root).length],
        [pollsWhileUnfetched + 1, 1],
        reason: 'ready and unfetched: the next interval polls again and fetches the file',
      );
      final pollsWhenFetched = desk.examGetCalls;
      await tester.pump(const Duration(seconds: 30));
      expect(
        desk.examGetCalls,
        pollsWhenFetched,
        reason: 'fetched: three more intervals send no poll beyond the $pollsWhenFetched before',
      );
    },
  );

  testWidgets(
    '"Exam ready" appears when the held file is written, not when a poll first shows ready',
    (tester) async {
      final fixture = await examFixture('exam-ready-when-written');
      final desk = fetchingDesk(carried: [null, contract('CarriedReplyDto')]);
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await openDeck(tester);
      await tapGenerate(tester);
      await closeSheet(tester);

      await tick(tester);
      expect(
        [find.text('Exam ready: Remote exam.').evaluate().length, heldFiles(fixture.root).length],
        [0, 0],
        reason: 'first ready poll without a document: no announcement and no file',
      );
      await tick(tester);
      expect(
        [find.text('Exam ready: Remote exam.').evaluate().length, heldFiles(fixture.root).length],
        [1, 1],
        reason: 'the file is written: the announcement shows once',
      );
    },
  );

  testWidgets(
    'the announcement comes once when the claim before a start, not a poll, fetched the exam',
    (tester) async {
      final fixture = await examFixture('exam-ready-by-claim');
      writeExamDeck(fixture.root, 'other.md', 'Other exam');
      final desk = FakeServerClient(
        versionReply: minServerVersion,
        idleBeforeStart: true,
        examStartSittings: [corpusSitting(), 'sitting-of-the-other-deck'],
        examGetReplies: [deskState('generating', corpusSitting())],
        examCarriedReply: contract('CarriedReplyDto'),
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await openDeck(tester);
      await tapGenerate(tester);
      await closeSheet(tester);
      desk.replaceExam(deskState('ready', corpusSitting()));
      final pollsBefore = desk.examGetAnswered;

      await openDeck(tester, 'Other exam');
      await tapGenerate(tester);
      expect(
        [desk.examGetAnswered - pollsBefore, desk.carriedSittings.length, heldFiles(fixture.root).length, desk.examStartCalls],
        [1, 1, 1, 2],
        reason: 'the claim was the one exam read (no poll ran), it made one fetch, one file is held, then the other deck started',
      );
      await tester.pump();
      expect(
        find.text('Exam ready: Remote exam.'),
        findsOneWidget,
        reason: 'the record the claim fetched is announced',
      );
      await closeSheet(tester);
      await tick(tester);
      await tick(tester);
      await tester.pumpAndSettle();
      expect(
        find.text('Exam ready: Remote exam.'),
        findsNothing,
        reason: 'two ready polls after the claim announce nothing a second time',
      );
    },
  );

  testWidgets(
    'a blocked claim shows the prepared-exam line and sends no start until Discard exam',
    (tester) async {
      final fixture = await examFixture('exam-claim-blocked');
      writeExamDeck(fixture.root, 'other.md', 'Other exam');
      final desk = FakeServerClient(
        versionReply: minServerVersion,
        idleBeforeStart: true,
        examStartSitting: corpusSitting(),
        examGetReplies: [deskState('ready', corpusSitting())],
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await openDeck(tester);
      await tapGenerate(tester);
      await closeSheet(tester);
      await tick(tester);
      expect(heldFiles(fixture.root), isEmpty, reason: 'step 1: the document never arrives');

      await openDeck(tester, 'Other exam');
      await tapGenerate(tester);
      await tester.pumpAndSettle();
      expect(
        [desk.examStartCalls, desk.startedDeck],
        [1, 'ws/exam.md'],
        reason: 'step 2: the blocked claim sends no start for the other deck',
      );
      expect(
        find.text('could not fetch the exam for Remote exam'),
        findsOneWidget,
        reason: 'step 2: the row reads could not fetch the exam for Remote exam',
      );

      await chooseDiscard(tester);
      expect(
        find.descendant(of: find.byType(AlertDialog), matching: find.text('Remote exam')),
        findsOneWidget,
        reason: 'step 3: the confirm opened from Other exam names Remote exam, whose exam it discards',
      );
      await tester.tap(
        find.descendant(of: find.byType(AlertDialog), matching: find.text('Discard exam')),
      );
      await tester.pumpAndSettle();
      expect(
        [desk.examClosedSittings, desk.examStartCalls, desk.startedDeck],
        [[corpusSitting()], 2, 'ws/other.md'],
        reason: 'step 4: the ready exam is closed, then the held-back start is sent',
      );
    },
  );

  testWidgets(
    'Discard exam asks first, then flags the record, removes the file and sends both closes',
    (tester) async {
      final fixture = await examFixture('exam-discard');
      final desk = fetchingDesk();
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await generateAndFetch(tester);
      expect(heldFiles(fixture.root), hasLength(1), reason: 'step 1: the exam is held');
      forgetFetched(fixture.support);
      desk.replaceExam(idleDesk);
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      final closedBefore = desk.examClosedSittings.length;

      await openDeck(tester);
      expect(find.text('Take exam'), findsOneWidget, reason: 'step 2: the held exam is offered');
      await chooseDiscard(tester);
      expect(
        [
          find.descendant(of: find.byType(AlertDialog), matching: find.text('Remote exam')).evaluate().length,
          heldFiles(fixture.root).length,
          records(fixture.support).single['fetched'],
        ],
        [1, 1, false],
        reason: 'step 3: the confirm names Remote exam before anything changes, the file held and the record unflagged',
      );
      await tester.tap(
        find.descendant(of: find.byType(AlertDialog), matching: find.text('Discard exam')),
      );
      await tester.pumpAndSettle();
      expect(
        [
          heldFiles(fixture.root).length,
          records(fixture.support).single['fetched'],
          desk.examClosedSittings.skip(closedBefore).toList(),
          desk.gradingClosedSittings,
        ],
        [0, true, [corpusSitting()], [corpusSitting()]],
        reason: 'step 4: confirmed, the record is flagged, the file is gone, both desks are closed',
      );
      expect(
        [find.text('Take exam').evaluate().length, find.text('Generate exam').evaluate().length],
        [0, 1],
        reason: 'step 5: the open sheet re-reads the held state: no Take exam, one Generate exam',
      );
    },
  );

  testWidgets('with another pairing active, Discard exam sends no close', (tester) async {
    final fixture = await examFixture('exam-discard-other-pairing');
    final desk = fetchingDesk();
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
    await generateAndFetch(tester);
    await savePairing(pairing(otherRootId), support: fixture.support);
    final other = FakeServerClient(versionReply: minServerVersion, examGetReplies: const [idleDesk]);
    await pumpExamPicker(
      tester,
      root: fixture.root,
      support: fixture.support,
      deskFor: (config) => config.rootId == pairedRootId ? desk : other,
    );
    final closedBefore = desk.examClosedSittings.length;

    await openDeck(tester);
    await confirmDiscard(tester);
    expect(
      [
        heldFiles(fixture.root).length,
        desk.examClosedSittings.length - closedBefore,
        desk.gradingClosedSittings.length,
        other.examClosedSittings.length,
        other.gradingClosedSittings.length,
      ],
      [0, 0, 0, 0, 0],
      reason: 'the file is removed and neither desktop is sent a close',
    );
  });

  testWidgets('after a discard the deck offers Generate exam again', (tester) async {
    final fixture = await examFixture('exam-discard-generate');
    final desk = fetchingDesk();
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
    await generateAndFetch(tester);
    await openDeck(tester);
    expect(find.text('Take exam'), findsOneWidget, reason: 'before: the held exam is offered');

    await confirmDiscard(tester);
    expect(
      [find.text('Take exam').evaluate().length, find.text('Generate exam').evaluate().length],
      [0, 1],
      reason: 'after: the row offers a new generation',
    );
    expect(find.text('graded by your paired desktop'), findsOneWidget, reason: 'after: the generate hint');
  });

  testWidgets(
    'leaving a result that has an outcome removes the file; one whose apply failed keeps it',
    (tester) async {
      final applied = await examFixture('exam-leave-result');
      final desk = fetchingDesk();
      await pumpExamPicker(tester, root: applied.root, support: applied.support, deskFor: (_) => desk);
      await generateAndFetch(tester);
      storeResult(applied.deck, applied.root);
      await openDeck(tester);
      await tester.tap(find.text('Show result'));
      await tester.pumpAndSettle();
      expect(find.text('not recorded'), findsOneWidget, reason: 'outcome: the applied result shows');
      await leaveExam(tester);
      expect(
        [heldFiles(applied.root).length, records(applied.support).single['fetched']],
        [0, true],
        reason: 'outcome: leaving removes the file and the record stays flagged',
      );

      final unapplied = await examFixture('exam-leave-unapplied');
      final second = fetchingDesk();
      await pumpExamPicker(tester, root: unapplied.root, support: unapplied.support, deskFor: (_) => second);
      await generateAndFetch(tester);
      storeResult(unapplied.deck, unapplied.root);
      await openDeck(tester);
      File(unapplied.deck).deleteSync();
      await tester.tap(find.text('Show result'));
      await tester.pumpAndSettle();
      expect(find.text('try again'), findsOneWidget, reason: 'failed apply: the screen offers a retry');
      await leaveExam(tester);
      expect(heldFiles(unapplied.root), hasLength(1), reason: 'failed apply: leaving keeps the file');
    },
  );

  testWidgets('a deck with a held exam opens it with the desktop unreachable', (tester) async {
    final fixture = await examFixture('exam-held-offline');
    final desk = fetchingDesk();
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
    await generateAndFetch(tester);
    final offline = FakeServerClient();
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => offline);

    await openDeck(tester);
    expect(
      [find.text('Take exam').evaluate().length, find.text('grading needs your paired desktop').evaluate().length],
      [1, 1],
      reason: 'offline: the held exam is offered with the grading hint',
    );
    await tester.tap(find.text('Take exam'));
    await tester.pumpAndSettle();
    expect(
      find.text('Why does Rust use ownership?'),
      findsOneWidget,
      reason: 'offline: the exam screen shows the held question',
    );
  });

  testWidgets(
    'a generation started on one pairing is fetched after switching the active pairing away and back',
    (tester) async {
      final fixture = await examFixture('exam-switch-pairing');
      final desk = FakeServerClient(
        versionReply: minServerVersion,
        idleBeforeStart: true,
        examStartSitting: corpusSitting(),
        examGetReplies: [deskState('generating', corpusSitting())],
        examCarriedReply: contract('CarriedReplyDto'),
      );
      final other = FakeServerClient(versionReply: minServerVersion, examGetReplies: const [idleDesk]);
      FakeServerClient deskFor(ServerConfig config) => config.rootId == pairedRootId ? desk : other;
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: deskFor);
      await openDeck(tester);
      await tapGenerate(tester);
      await closeSheet(tester);

      await savePairing(pairing(otherRootId), support: fixture.support);
      final otherRoot = pairedWorkspace(fixture.support, otherRootId);
      desk.replaceExam(deskState('ready', corpusSitting()));
      await pumpExamPicker(tester, root: otherRoot, support: fixture.support, deskFor: deskFor);
      expect(
        [desk.carriedSittings.length, heldFiles(fixture.root).length],
        [0, 0],
        reason: 'away: the other pairing is polled, the first desk is not fetched',
      );

      await setActiveRoot(pairedRootId, support: fixture.support);
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: deskFor);
      expect(
        [heldFiles(fixture.root).length, find.text('Exam ready: Remote exam.').evaluate().length],
        [1, 1],
        reason: 'back: the first desk is polled once and its ready exam is fetched',
      );
    },
  );

  testWidgets(
    'a claim whose poll gets no reply sends no start and shows the notice of a start without a reply',
    (tester) async {
      final fixture = await examFixture('exam-claim-unreachable');
      final gates = <int, Completer<RemoteExam?>>{};
      final desk = FakeServerClient(
        versionReply: minServerVersion,
        examStartSitting: corpusSitting(),
        examGetReplies: const [idleDesk],
        examGetGates: gates,
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      gates[desk.examGetCalls] = Completer<RemoteExam?>()..complete(null);

      await openDeck(tester);
      await tapGenerate(tester);
      await tester.pumpAndSettle();
      expect(desk.examStartCalls, 0, reason: 'no reply to the claim: no start is sent');
      expect(
        find.descendant(
          of: find.byType(PickerDepthSheet),
          matching: find.text('the desktop refused the exam'),
        ),
        findsOneWidget,
        reason: 'the row shows the notice of a start without a reply',
      );
    },
  );

  testWidgets('a declined discard confirm discards nothing and sends nothing', (tester) async {
    final fixture = await examFixture('exam-discard-declined');
    final desk = fetchingDesk();
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
    await generateAndFetch(tester);
    final closedBefore = desk.examClosedSittings.length;

    await openDeck(tester);
    await chooseDiscard(tester);
    await tester.tap(find.descendant(of: find.byType(AlertDialog), matching: find.text('Keep')));
    await tester.pumpAndSettle();
    expect(
      [
        heldFiles(fixture.root).length,
        desk.examClosedSittings.length - closedBefore,
        desk.gradingClosedSittings.length,
        find.text('Take exam').evaluate().length,
      ],
      [1, 0, 0, 1],
      reason: 'declined: the file stays, no close is sent, the row still offers the exam',
    );
  });

  testWidgets(
    'the discard returns while both closes are unanswered, and a close that fails afterwards raises nothing',
    (tester) async {
      final fixture = await examFixture('exam-discard-unanswered');
      final examClose = Completer<void>();
      final gradingClose = Completer<void>();
      final desk = fetchingDesk(
        examCloseGate: examClose,
        examCloseReply: false,
        gradingCloseGate: gradingClose,
        expireOnGradingClose: true,
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await generateAndFetch(tester);
      final closedBefore = desk.examClosedSittings.length;

      await openDeck(tester);
      await tester.tap(find.text('Take exam'));
      await tester.pumpAndSettle();
      await tester.tap(find.descendant(of: find.byType(ExamScreen), matching: find.text('Discard exam')));
      await tester.pumpAndSettle();
      await tester.tap(find.descendant(of: find.byType(AlertDialog), matching: find.text('Discard exam')));
      await tester.pumpAndSettle();
      expect(
        [
          find.byType(ExamScreen).evaluate().length,
          heldFiles(fixture.root).length,
          desk.examClosedSittings.length - closedBefore,
          desk.gradingClosedSittings,
        ],
        [0, 0, 1, [corpusSitting()]],
        reason: 'closes in flight: the screen has left, the file is gone, both closes are sent',
      );
      await openDeck(tester);
      expect(find.text('Generate exam'), findsOneWidget, reason: 'closes in flight: the row offers a generation');

      examClose.complete();
      gradingClose.complete();
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull, reason: 'a refused and an expired close raise nothing');
    },
  );

  testWidgets(
    'a deck file renamed after its exam was fetched still shows Take exam and opens it',
    (tester) async {
      final fixture = await examFixture('exam-renamed-deck');
      final desk = fetchingDesk();
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await generateAndFetch(tester);
      File(fixture.deck).renameSync('${fixture.root.path}/ws/decks/renamed.md');
      desk.replaceExam(idleDesk);
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);

      await openDeck(tester);
      expect(find.text('Take exam'), findsOneWidget, reason: 'renamed: the held exam is still offered');
      await tester.tap(find.text('Take exam'));
      await tester.pumpAndSettle();
      expect(
        find.text('Why does Rust use ownership?'),
        findsOneWidget,
        reason: 'renamed: the exam opens on its question',
      );
    },
  );

  testWidgets('a deck whose id cannot be read offers Generate exam', (tester) async {
    final fixture = await examFixture('exam-unreadable-id');
    final desk = fetchingDesk();
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
    await generateAndFetch(tester);
    final deck = File(fixture.deck);
    deck.writeAsStringSync(
      deck.readAsStringSync().replaceFirst(RegExp(r'^id: .*\n', multiLine: true), ''),
    );

    await openDeck(tester);
    expect(
      [heldFiles(fixture.root).length, find.text('Generate exam').evaluate().length, find.text('Take exam').evaluate().length],
      [1, 1, 0],
      reason: 'a file is held, but a deck that declares no id offers a generation',
    );
  });

  testWidgets(
    'a held file that cannot be read shows Take exam with Discard exam, and its discard removes it and sends nothing',
    (tester) async {
      final fixture = await examFixture('exam-unreadable-held');
      final desk = fetchingDesk();
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await generateAndFetch(tester);
      heldFiles(fixture.root).single.writeAsStringSync('not an exam');
      forgetFetched(fixture.support);
      desk.replaceExam(idleDesk);
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      final closedBefore = desk.examClosedSittings.length;

      await openDeck(tester);
      expect(
        [find.text('Take exam').evaluate().length, find.byIcon(Icons.more_vert).evaluate().length],
        [1, 1],
        reason: 'unreadable: the row offers the exam and a discard',
      );
      await confirmDiscard(tester);
      expect(
        [
          heldFiles(fixture.root).length,
          desk.examClosedSittings.length - closedBefore,
          desk.gradingClosedSittings.length,
          records(fixture.support).single['fetched'],
        ],
        [0, 0, 0, false],
        reason: 'unreadable discard: the file is removed, nothing is sent, no record is flagged',
      );
    },
  );

  testWidgets(
    'the exam screen gets a client of its own, and the picker polling client stays open after the screen closed it',
    (tester) async {
      final fixture = await examFixture('exam-screen-client');
      final desk = fetchingDesk();
      final handles = await pumpExamPicker(
        tester,
        root: fixture.root,
        support: fixture.support,
        deskFor: (_) => desk,
      );
      await generateAndFetch(tester);
      await openDeck(tester);
      final builtBefore = handles.length;
      await tester.tap(find.text('Take exam'));
      await tester.pumpAndSettle();
      final screenHandles = handles.skip(builtBefore).toList();
      await leaveExam(tester);

      expect(
        [screenHandles.length, screenHandles.single.closed],
        [1, true],
        reason: 'the screen was handed one client built for it, and it closed it',
      );
      expect(
        handles.where((handle) => handle.examGets > 0 && handle.closed),
        isEmpty,
        reason: 'the client that polls the desk is still open',
      );
    },
  );

  testWidgets(
    'the deck sheet reads the held file when it opens and after a hand-off attempt or a start check, never because it rebuilt',
    (tester) async {
      final fixture = await examFixture('exam-held-reads');
      final desk = FakeServerClient(
        versionReply: minServerVersion,
        idleBeforeStart: true,
        examStartSitting: corpusSitting(),
        examGetReplies: [deskState('generating', corpusSitting())],
        examCarriedReply: contract('CarriedReplyDto'),
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      final before = exam_bridge.heldExamViewReads;
      await openDeck(tester);
      expect(exam_bridge.heldExamViewReads - before, 1, reason: 'opening: one read');
      await tapGenerate(tester);
      final afterStart = exam_bridge.heldExamViewReads;
      expect(
        afterStart - before,
        3,
        reason: 'the claim: one read by the sheet on the bump, one by the start for this deck',
      );

      await tick(tester);
      await tick(tester);
      await tick(tester);
      expect(
        exam_bridge.heldExamViewReads,
        afterStart,
        reason: 'three generating polls attempt no hand-off and rebuild the sheet without a read',
      );
      desk.replaceExam(deskState('ready', corpusSitting()));
      await tick(tester);
      expect(
        [exam_bridge.heldExamViewReads - afterStart, find.text('Take exam').evaluate().length],
        [1, 1],
        reason: 'the hand-off attempt that fetched: one read, and the row offers the exam',
      );
    },
  );

  testWidgets(
    'a discard whose record cannot be saved keeps the held file and shows the error in the row',
    (tester) async {
      final fixture = await examFixture('exam-discard-unsaved');
      final desk = fetchingDesk();
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await generateAndFetch(tester);
      forgetFetched(fixture.support);
      desk.replaceExam(idleDesk);
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      blockRecordSaves(fixture.support);

      await openDeck(tester);
      await confirmDiscard(tester);
      expect(
        [heldFiles(fixture.root).length, find.text('Take exam').evaluate().length, find.text('graded by your paired desktop').evaluate().length],
        [1, 1, 0],
        reason: 'the record could not be flagged: the file stays and the row shows the error, not its hint',
      );
    },
  );

  testWidgets('leaving a result whose record cannot be saved keeps the held file', (tester) async {
    final fixture = await examFixture('exam-leave-unsaved');
    final desk = fetchingDesk();
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
    await generateAndFetch(tester);
    storeResult(fixture.deck, fixture.root);
    forgetFetched(fixture.support);
    desk.replaceExam(idleDesk);
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
    blockRecordSaves(fixture.support);

    await openDeck(tester);
    await tester.tap(find.text('Show result'));
    await tester.pumpAndSettle();
    await leaveExam(tester);
    expect(heldFiles(fixture.root), hasLength(1), reason: 'the record could not be flagged: the file stays');
  });

  testWidgets('a ready exam this phone did not start is neither fetched nor announced', (tester) async {
    final fixture = await examFixture('exam-ready-elsewhere');
    final desk = FakeServerClient(
      versionReply: minServerVersion,
      examGetReplies: [deskState('ready', 'a-sitting-another-client-started')],
      examCarriedReply: contract('CarriedReplyDto'),
    );
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
    await tester.pump();

    expect(
      [desk.carriedSittings.length, heldFiles(fixture.root).length, find.textContaining('Exam ready').evaluate().length],
      [0, 0, 0],
      reason: 'no record names the sitting: nothing is fetched or announced',
    );
    await openDeck(tester);
    expect(find.text('Generate exam'), findsOneWidget, reason: 'the row offers a generation');
  });

  testWidgets(
    'a phone-started preparation is fetched and announced after an app restart',
    (tester) async {
      final fixture = await examFixture('exam-ready-after-restart');
      final before = FakeServerClient(
        versionReply: minServerVersion,
        idleBeforeStart: true,
        examStartSitting: corpusSitting(),
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => before);
      await openDeck(tester);
      await tapGenerate(tester);
      expect(before.startedDeck, 'ws/exam.md', reason: 'step 1: the phone owns this preparation');

      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();
      StartedSittings.forgetLoaded();
      final after = FakeServerClient(
        versionReply: minServerVersion,
        examGetReplies: [deskState('ready', corpusSitting())],
        examCarriedReply: contract('CarriedReplyDto'),
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => after);
      expect(
        [heldFiles(fixture.root).length, find.text('Exam ready: Remote exam.').evaluate().length],
        [1, 1],
        reason: 'step 2: the restarted picker fetches and announces the exam this phone started',
      );
    },
  );

  testWidgets('two live pickers announce one fetched exam only once', (tester) async {
    final fixture = await examFixture('exam-ready-two-pickers');
    final starter = FakeServerClient(
      versionReply: minServerVersion,
      idleBeforeStart: true,
      examStartSitting: corpusSitting(),
    );
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => starter);
    await openDeck(tester);
    await tapGenerate(tester);
    await tester.pumpWidget(const SizedBox.shrink());
    StartedSittings.forgetLoaded();

    final shared = FakeServerClient(
      versionReply: minServerVersion,
      examGetReplies: [deskState('ready', corpusSitting())],
      examCarriedReply: contract('CarriedReplyDto'),
    );
    Widget picker() => Expanded(
      child: ScaffoldMessenger(
        child: PickerScreen(
          root: fixture.root.path,
          dir: '${fixture.root.path}/ws',
          supportDir: fixture.support,
          buildClient: (_) => _DeskHandle(shared),
          generatePollInterval: const Duration(seconds: 10),
          isPairedSubtree: true,
        ),
      ),
    );
    await tester.pumpWidget(
      MaterialApp(theme: alixDark(), home: Column(children: [picker(), picker()])),
    );
    await tester.pumpAndSettle();

    expect(
      [heldFiles(fixture.root).length, find.text('Exam ready: Remote exam.').evaluate().length],
      [1, 1],
      reason: 'one file is held and only one of the two pickers announces it',
    );
  });

  testWidgets(
    'a preparing exam is cancelled from its row after confirmation',
    (tester) async {
      final fixture = await examFixture('exam-cancel');
      final desk = FakeServerClient(
        versionReply: minServerVersion,
        idleBeforeStart: true,
        examStartSitting: corpusSitting(),
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await openDeck(tester);
      await tapGenerate(tester);
      expect(
        find.textContaining('preparing…'),
        findsOneWidget,
        reason: 'step 1: after the start, one row reads preparing',
      );

      await tester.tap(find.text('Generate exam'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Keep'));
      await tester.pumpAndSettle();
      expect(desk.examClosedSittings, isEmpty, reason: 'step 2: Keep keeps it');
      expect(
        find.textContaining('preparing…'),
        findsOneWidget,
        reason: 'step 2: after Keep, one row still reads preparing',
      );

      await tester.tap(find.text('Generate exam'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Cancel preparation'));
      await tester.pumpAndSettle();
      expect(
        [desk.examClosedSittings, records(fixture.support)],
        [[corpusSitting()], isEmpty],
        reason: 'step 3: the desktop is told which sitting, and the record is removed',
      );
      expect(
        find.text('graded by your paired desktop'),
        findsOneWidget,
        reason: 'step 3: the row offers a fresh generation',
      );
    },
  );

  testWidgets(
    'an unanswered poll while preparing keeps the exam and keeps polling',
    (tester) async {
      final fixture = await examFixture('exam-unanswered-poll');
      final unanswered = Completer<RemoteExam?>();
      final desk = FakeServerClient(
        versionReply: minServerVersion,
        idleBeforeStart: true,
        examStartSitting: corpusSitting(),
        examGetReplies: [
          deskState('generating', corpusSitting()),
          deskState('generating', corpusSitting()),
          deskState('ready', corpusSitting()),
        ],
        examGetGates: {1: unanswered},
        examCarriedReply: contract('CarriedReplyDto'),
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await openDeck(tester);
      await tapGenerate(tester);
      await closeSheet(tester);
      await tick(tester);
      await tester.pump(const Duration(seconds: 10));
      expect(desk.examGetCalls, 2, reason: 'step 1: the second poll is in flight');
      unanswered.complete(null);
      await tester.pump();
      await tester.pump();

      await openDeck(tester);
      expect(
        find.textContaining('preparing…'),
        findsOneWidget,
        reason: 'step 2: a poll the desktop did not answer is not an empty slot',
      );

      await tick(tester);
      expect(desk.examGetCalls, 3, reason: 'step 3: polling continues');
      expect(find.text('Take exam'), findsOneWidget, reason: 'step 3: the fetched exam shows');
    },
  );

  testWidgets(
    'an unanswered resume poll while preparing restarts periodic polling',
    (tester) async {
      final root = tempDir('alix-picker-exam-unanswered-resume-poll-');
      final support = tempDir('alix-picker-exam-unanswered-resume-poll-support-');
      Directory('${root.path}/ws/decks').createSync(recursive: true);
      File('${root.path}/ws/alix.toml').writeAsStringSync('title = "Workspace"\n');
      await savePairing(pairing(pairedRootId), support: support);
      final unanswered = Completer<RemoteExam?>();
      final rootExamClient = FakeServerClient(
        examGetReplies: [deskState('generating', 'sitting-1'), deskState('ready', 'sitting-1')],
        examGetGates: {1: unanswered},
      );
      final clients = <FakeServerClient>[
        FakeServerClient(versionReply: minServerVersion),
        rootExamClient,
        FakeServerClient(versionReply: minServerVersion),
        FakeServerClient(),
        FakeServerClient(versionReply: minServerVersion),
      ];
      var clientIndex = 0;
      await pumpPicker(
        tester,
        root: root,
        support: support,
        buildClient: (_) => clients[clientIndex++],
        pollInterval: const Duration(seconds: 10),
        buildSyncPort: (_, _) => FakeSyncPort(rootId: pairedRootId, rootDir: root.path),
        isPairedSubtree: true,
      );
      expect(rootExamClient.examGetCalls, 1, reason: 'step 1: preparing');

      await tester.tap(find.text('Workspace'));
      await tester.pumpAndSettle();
      await tester.tap(find.byType(BackButton));
      await tester.pumpAndSettle();
      expect(rootExamClient.examGetCalls, 2, reason: 'step 2: resume poll');

      unanswered.complete(null);
      await tester.pump();
      await tester.pump(const Duration(seconds: 10));
      await tester.pump();
      expect(
        rootExamClient.examGetCalls,
        3,
        reason: 'step 3: unanswered resume poll must restart the timer',
      );
    },
  );

  testWidgets('an older picker poll cannot erase a newly started exam', (
    tester,
  ) async {
    final fixture = await examFixture('exam-start-poll-race');
    final stalePoll = Completer<RemoteExam?>();
    final desk = FakeServerClient(
      versionReply: minServerVersion,
      examStartSitting: corpusSitting(),
      examGetReplies: const [idleDesk],
      examGetGates: {0: stalePoll},
    );
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
    expect(desk.examGetCalls, 1, reason: 'the older empty-slot poll is in flight');

    await openDeck(tester);
    await tapGenerate(tester);
    expect(
      find.textContaining('preparing…'),
      findsOneWidget,
      reason: 'started: one row reads preparing before the stale reply lands',
    );

    stalePoll.complete(idleDesk);
    await tester.pump();
    await tester.pump();
    expect(
      find.textContaining('preparing…'),
      findsOneWidget,
      reason: 'an older empty-slot reply must not hide the accepted sitting',
    );
  });

  testWidgets(
    'a fresh poll after the stale start reply can still fetch the ready exam',
    (tester) async {
      final fixture = await examFixture('exam-start-fresh-poll');
      final stalePoll = Completer<RemoteExam?>();
      final desk = FakeServerClient(
        versionReply: minServerVersion,
        examStartSitting: corpusSitting(),
        examGetReplies: [idleDesk, idleDesk, deskState('ready', corpusSitting())],
        examGetGates: {0: stalePoll},
        examCarriedReply: contract('CarriedReplyDto'),
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      expect(desk.examGetCalls, 1, reason: 'the pre-start poll is in flight');

      await openDeck(tester);
      await tapGenerate(tester);
      expect(
        find.textContaining('preparing…'),
        findsOneWidget,
        reason: 'started: one row reads preparing before the stale reply lands',
      );

      stalePoll.complete(null);
      await tester.pump();
      await tick(tester);

      expect(desk.examGetCalls, 3, reason: 'the stale poll, the claim, and the post-start poll');
      expect(
        find.text('Take exam'),
        findsOneWidget,
        reason: 'the fresh ready reply is fetched after the stale reply is dropped',
      );
    },
  );

  testWidgets('an older picker poll cannot resurrect a cancelled exam', (
    tester,
  ) async {
    final fixture = await examFixture('exam-cancel-poll-race');
    final stalePoll = Completer<RemoteExam?>();
    final desk = FakeServerClient(
      versionReply: minServerVersion,
      examStartSitting: corpusSitting(),
      examGetReplies: const [idleDesk],
      examGetGates: {2: stalePoll},
    );
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
    expect(desk.examGetCalls, 1, reason: 'the initial empty-slot poll completed');

    await openDeck(tester);
    await tapGenerate(tester);
    await tester.pump(const Duration(seconds: 10));
    expect(desk.examGetCalls, 3, reason: 'the pre-cancel poll is in flight');

    await tester.tap(find.text('Generate exam'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Cancel preparation'));
    await tester.pumpAndSettle();
    expect(
      find.text('graded by your paired desktop'),
      findsOneWidget,
      reason: 'cancelled: one row offers a fresh generation before the stale reply lands',
    );

    stalePoll.complete(deskState('generating', corpusSitting()));
    await tester.pump();
    await tester.pump();
    expect(
      find.text('graded by your paired desktop'),
      findsOneWidget,
      reason: 'a reply issued before cancellation must not restore preparation',
    );
    final pollsAfterCancel = desk.examGetCalls;
    await tester.pump(const Duration(seconds: 30));
    expect(
      desk.examGetCalls,
      pollsAfterCancel,
      reason: 'a reply issued before cancellation must not restart polling ($pollsAfterCancel polls before)',
    );
  });

  testWidgets('an expired pairing at exam start explains how to recover', (
    tester,
  ) async {
    final fixture = await examFixture('exam-start-expired');
    final desk = FakeServerClient(
      versionReply: minServerVersion,
      examGetReplies: const [idleDesk],
      expireOnExamStart: true,
    );
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);

    await openDeck(tester);
    await tester.tap(find.text('Generate exam'));
    await tester.pumpAndSettle();

    expect(
      find.text(
        'Pairing expired. Pair again from Settings → Connected devices.',
      ),
      findsOneWidget,
      reason: 'expired start: one pairing-expired line is shown',
    );
  });

  testWidgets(
    'a refused exam start shows in the open deck sheet, not under it',
    (tester) async {
      final fixture = await examFixture('exam-start-refused');
      final desk = FakeServerClient(
        versionReply: minServerVersion,
        examGetReplies: const [idleDesk],
        examStartReply: false,
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);

      await openDeck(tester);
      await tester.tap(find.text('Generate exam'));
      await tester.pumpAndSettle();

      expect(
        find.descendant(
          of: find.byType(PickerDepthSheet),
          matching: find.text('the desktop refused the exam'),
        ),
        findsOneWidget,
        reason: 'a root SnackBar renders beneath the modal sheet',
      );

      Navigator.of(tester.element(find.byType(PickerDepthSheet))).pop();
      await tester.pumpAndSettle();
      await openDeck(tester);
      expect(
        find.text('the desktop refused the exam'),
        findsNothing,
        reason: 'reopening the sheet starts without the old refusal',
      );
    },
  );

  testWidgets('a start reply that names no sitting is refused and puts no start record', (tester) async {
    final fixture = await examFixture('exam-start-without-sitting');
    final desk = FakeServerClient(
      versionReply: minServerVersion,
      idleBeforeStart: true,
    );
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);

    await openDeck(tester);
    await tapGenerate(tester);
    await tester.pumpAndSettle();
    expect(
      [desk.examStartCalls, find.text('the desktop refused the exam').evaluate().length, records(fixture.support)],
      [1, 1, isEmpty],
      reason: 'a reply without its sitting is a refused start and leaves no record',
    );
  });

  testWidgets(
    'a successful cancel retry clears its earlier row failure',
    (tester) async {
      final fixture = await examFixture('exam-cancel-retry');
      final desk = _FailFirstExamCloseClient(sitting: corpusSitting());
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);

      await openDeck(tester);
      await tapGenerate(tester);

      await tester.tap(find.text('Generate exam'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Cancel preparation'));
      await tester.pumpAndSettle();
      expect(
        find.text('could not cancel the preparation'),
        findsOneWidget,
        reason: 'first cancel fails: one row reads could not cancel the preparation',
      );

      await tester.tap(find.text('Generate exam'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Cancel preparation'));
      await tester.pumpAndSettle();
      expect(desk.examCloseCalls, 2, reason: 'the retry reached the desktop');
      expect(
        find.text('could not cancel the preparation'),
        findsNothing,
        reason: 'the successful retry must clear the obsolete failure',
      );
      expect(
        find.text('graded by your paired desktop'),
        findsOneWidget,
        reason: 'a successful retry must clear the obsolete failure',
      );
    },
  );

  testWidgets('a cancel whose failed answer arrives after the picker is gone writes nothing to it', (tester) async {
    final fixture = await examFixture('exam-cancel-after-dispose');
    final gate = Completer<void>();
    final desk = FakeServerClient(
      versionReply: minServerVersion,
      idleBeforeStart: true,
      examStartSitting: corpusSitting(),
      examCloseGate: gate,
      examCloseReply: false,
    );
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
    await openDeck(tester);
    await tapGenerate(tester);
    await tester.tap(find.text('Generate exam'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Cancel preparation'));
    await tester.pump();
    expect(desk.examCloseCalls, 1, reason: 'confirmed cancel: the close is waiting at the desktop');

    await tester.pumpWidget(const SizedBox.shrink());
    gate.complete();
    await tester.pump();

    expect(tester.takeException(), isNull, reason: 'refused close after the picker is gone: nothing throws');
  });

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
      expect(
        find.byType(FolderBrowser),
        findsOneWidget,
        reason: 'generated: one folder browser opens for the destination',
      );
      await expectWidgetTree(
        tester,
        'picker_generate_destination',
        root: find.byType(FolderBrowser),
      );
    },
  );
  const expiredLine = 'Pairing expired. Pair again from Settings → Connected devices.';

  Future<void> startThenRestart(WidgetTester tester, ({Directory root, Directory support, String deck}) fixture) async {
    final starter = FakeServerClient(
      versionReply: minServerVersion,
      idleBeforeStart: true,
      examStartSitting: corpusSitting(),
    );
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => starter);
    await openDeck(tester);
    await tapGenerate(tester);
    await tester.pumpWidget(const SizedBox.shrink());
    StartedSittings.forgetLoaded();
  }

  testWidgets(
    "a claim that fetches this deck's own exam sends no start and offers the exam",
    (tester) async {
      final fixture = await examFixture('exam-claim-own');
      await startThenRestart(tester, fixture);
      final desk = FakeServerClient(
        versionReply: minServerVersion,
        nullFirstExamGetCalls: 1,
        examGetReplies: [deskState('ready', corpusSitting())],
        examCarriedReply: contract('CarriedReplyDto'),
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await openDeck(tester);
      expect(
        find.text('Generate exam'),
        findsOneWidget,
        reason: 'step 1: the first poll got no reply, so the row offers Generate exam',
      );

      await tapGenerate(tester);
      await tester.pump();
      expect(
        [
          desk.examStartCalls,
          heldFiles(fixture.root).length,
          find.text('Take exam').evaluate().length,
          find.text('Exam ready: Remote exam.').evaluate().length,
        ],
        [0, 1, 1, 1],
        reason: 'step 2: the claim fetched this deck exam: no start, one held file, Take exam, one announcement',
      );
    },
  );

  testWidgets(
    'a claim that fetches an exam the picker never polled announces it once',
    (tester) async {
      final fixture = await examFixture('exam-claim-unpolled');
      writeExamDeck(fixture.root, 'other.md', 'Other exam');
      await startThenRestart(tester, fixture);
      final desk = FakeServerClient(
        versionReply: minServerVersion,
        nullFirstExamGetCalls: 1,
        examStartSittings: const ['sitting-of-the-other-deck'],
        examGetReplies: [deskState('ready', corpusSitting())],
        examCarriedReply: contract('CarriedReplyDto'),
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);

      await openDeck(tester, 'Other exam');
      await tapGenerate(tester);
      await tester.pump();
      expect(
        [
          heldFiles(fixture.root).length,
          find.text('Exam ready: Remote exam.').evaluate().length,
          desk.startedDeck,
        ],
        [1, 1, 'ws/other.md'],
        reason: 'claim: the unpolled exam of Remote exam is held and announced once, then Other exam starts',
      );
      await closeSheet(tester);
      await tick(tester);
      await tester.pumpAndSettle();
      expect(
        find.text('Exam ready: Remote exam.'),
        findsNothing,
        reason: 'a later poll of the same ready exam announces nothing a second time',
      );
    },
  );

  testWidgets(
    'a poll whose hand-off meets an expired pairing after writing the file still announces it, and the open sheet offers Take exam',
    (tester) async {
      final fixture = await examFixture('exam-handoff-expired');
      final desk = FakeServerClient(
        versionReply: minServerVersion,
        idleBeforeStart: true,
        examStartSitting: corpusSitting(),
        examGetReplies: [deskState('ready', corpusSitting())],
        examCarriedReply: contract('CarriedReplyDto'),
        expireOnExamClose: true,
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await openDeck(tester);
      await tapGenerate(tester);

      await tick(tester);
      expect(
        [
          heldFiles(fixture.root).length,
          desk.examClosedSittings,
          find.text('Take exam').evaluate().length,
          find.text('Exam ready: Remote exam.').evaluate().length,
        ],
        [1, [corpusSitting()], 1, 1],
        reason: 'the file was written and its close expired: the sheet offers Take exam and the exam is announced once',
      );
    },
  );

  testWidgets(
    'a fetched exam that was never announced and is then discarded is not announced by the next hand-off',
    (tester) async {
      final fixture = await examFixture('exam-discard-unannounced');
      final desk = fetchingDesk();
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await generateAndFetch(tester);
      final file = File('${fixture.support.path}/exam-sittings.json');
      file.writeAsStringSync(
        file.readAsStringSync().replaceAll('"announced":true', '"announced":false'),
      );
      StartedSittings.forgetLoaded();
      desk.replaceExam(idleDesk);
      await tester.pumpWidget(const SizedBox.shrink());
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await openDeck(tester);
      await confirmDiscard(tester);
      await closeSheet(tester);
      expect(
        [heldFiles(fixture.root).length, find.text('Exam ready: Remote exam.').evaluate().length],
        [0, 0],
        reason: 'discarded: the file is gone and nothing is announced yet',
      );

      desk.replaceExam(deskState('ready', corpusSitting()));
      final closesBefore = desk.examClosedSittings.length;
      await tester.pumpWidget(const SizedBox.shrink());
      StartedSittings.forgetLoaded();
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await tester.pump();
      expect(
        desk.examClosedSittings.skip(closesBefore).toList(),
        [corpusSitting()],
        reason: 'the restarted poll handed the ready exam off: one new close of its sitting',
      );
      expect(
        find.text('Exam ready: Remote exam.'),
        findsNothing,
        reason: 'the next hand-off of the discarded exam announces nothing',
      );
    },
  );

  testWidgets(
    'a blocked claim is cleared once a poll fetches its exam while the sheet is open',
    (tester) async {
      final fixture = await examFixture('exam-blocked-cleared');
      writeExamDeck(fixture.root, 'other.md', 'Other exam');
      final desk = fetchingDesk(carried: [null, null, contract('CarriedReplyDto')]);
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await openDeck(tester);
      await tapGenerate(tester);
      await closeSheet(tester);
      await tick(tester);
      await openDeck(tester, 'Other exam');
      await tapGenerate(tester);
      await tester.pumpAndSettle();
      expect(
        find.byIcon(Icons.more_vert),
        findsOneWidget,
        reason: 'step 1: the blocked claim gives the Other exam row a discard menu',
      );

      await tick(tester);
      expect(
        [
          heldFiles(fixture.root).length,
          find.textContaining('could not').evaluate().length,
          find.byIcon(Icons.more_vert).evaluate().length,
          find.text('graded by your paired desktop').evaluate().length,
        ],
        [1, 0, 0, 1],
        reason: 'step 2: the poll fetched the exam: no blocked line, no discard menu, the generate hint',
      );
    },
  );

  testWidgets('a second start tap while a start is under way sends nothing', (tester) async {
    final fixture = await examFixture('exam-start-guard');
    final gates = <int, Completer<RemoteExam?>>{};
    final desk = FakeServerClient(
      versionReply: minServerVersion,
      examStartSitting: corpusSitting(),
      examGetReplies: const [idleDesk],
      examGetGates: gates,
      refuseStartWhileGenerating: true,
    );
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
    final claimCall = desk.examGetCalls;
    final claim = Completer<RemoteExam?>();
    gates[claimCall] = claim;

    await openDeck(tester);
    await tester.tap(find.text('Generate exam'));
    await tester.pump();
    await tester.tap(find.text('Generate exam'));
    await tester.pump();
    claim.complete(idleDesk);
    await tester.pump();
    await tester.pump();
    expect(
      [
        desk.examGetCalls - claimCall,
        desk.examStartCalls,
        find.textContaining('preparing…').evaluate().length,
        find.text('the desktop refused the exam').evaluate().length,
      ],
      [1, 1, 1, 0],
      reason: 'two taps on a desk that refuses a second start: one claim, one start, the row preparing without a refusal line',
    );
  });

  testWidgets(
    "the fetching row's Discard exam closes the ready exam and removes its record",
    (tester) async {
      final fixture = await examFixture('exam-fetching-discard');
      final desk = fetchingDesk(carried: [null]);
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await openDeck(tester);
      await tapGenerate(tester);
      await closeSheet(tester);
      await tick(tester);
      await openDeck(tester);
      expect(find.text('fetching…'), findsOneWidget, reason: 'step 1: the unfetched ready exam reads fetching');

      await confirmDiscard(tester);
      expect(
        [desk.examClosedSittings, records(fixture.support).length, find.text('graded by your paired desktop').evaluate().length],
        [[corpusSitting()], 0, 1],
        reason: 'step 2: the ready sitting is closed, its record removed, the row offers a generation',
      );
    },
  );

  testWidgets(
    'a failed generation of this phone is closed with its sitting before the next start',
    (tester) async {
      final fixture = await examFixture('exam-failed-closed');
      final desk = FakeServerClient(
        versionReply: minServerVersion,
        idleBeforeStart: true,
        examStartSittings: [corpusSitting(), 'the-next-sitting'],
        examGetReplies: [
          RemoteExam(
            sitting: corpusSitting(),
            phase: 'generating',
            deck: 'ws/exam.md',
            thinking: false,
            error: 'the generation failed',
          ),
        ],
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await openDeck(tester);
      await tapGenerate(tester);
      await closeSheet(tester);
      await tick(tester);
      await openDeck(tester);
      expect(
        find.text('the last preparation failed'),
        findsOneWidget,
        reason: 'step 1: the failed generation shows on the row',
      );

      await tapGenerate(tester);
      expect(
        [desk.examClosedSittings, desk.examStartCalls],
        [[corpusSitting()], 2],
        reason: 'step 2: the failed sitting is closed, then the second start is sent',
      );
    },
  );

  testWidgets('an expired pairing at the claim before a start explains how to recover', (tester) async {
    final fixture = await examFixture('exam-claim-expired');
    final desk = FakeServerClient(
      versionReply: minServerVersion,
      examStartSitting: corpusSitting(),
      examGetReplies: const [idleDesk],
      expireOnExamGetCall: 1,
    );
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
    await openDeck(tester);
    await tapGenerate(tester);
    await tester.pumpAndSettle();
    expect(
      [desk.examStartCalls, find.text(expiredLine).evaluate().length],
      [0, 1],
      reason: 'the claim poll expired: no start, one pairing-expired line',
    );
  });

  testWidgets(
    'an expired pairing while discarding a blocked claim explains how to recover',
    (tester) async {
      final fixture = await examFixture('exam-blocked-discard-expired');
      writeExamDeck(fixture.root, 'other.md', 'Other exam');
      final desk = FakeServerClient(
        versionReply: minServerVersion,
        idleBeforeStart: true,
        examStartSitting: corpusSitting(),
        examGetReplies: [deskState('ready', corpusSitting())],
        expireOnExamClose: true,
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await openDeck(tester);
      await tapGenerate(tester);
      await closeSheet(tester);
      await tick(tester);
      await openDeck(tester, 'Other exam');
      await tapGenerate(tester);
      await tester.pumpAndSettle();

      await confirmDiscard(tester);
      expect(
        [desk.examClosedSittings, desk.examStartCalls, find.text(expiredLine).evaluate().length],
        [[corpusSitting()], 1, 1],
        reason: 'the close expired: no held-back start, one pairing-expired line',
      );
    },
  );

  testWidgets('a deck whose file cannot be read when its sheet opens offers Generate exam', (tester) async {
    final fixture = await examFixture('exam-deck-unreadable');
    final desk = fetchingDesk();
    await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
    await generateAndFetch(tester);
    File(fixture.deck).deleteSync();

    await openDeck(tester);
    expect(
      [heldFiles(fixture.root).length, find.text('Generate exam').evaluate().length, find.text('Take exam').evaluate().length],
      [1, 1, 0],
      reason: 'a file is held, but a deck file deleted after the listing cannot be read and offers a generation',
    );
  });

  testWidgets(
    'a deck that holds an exam keeps its exam row after its sources are removed',
    (tester) async {
      final fixture = await examFixture('exam-held-not-examable');
      final desk = fetchingDesk();
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await generateAndFetch(tester);
      Directory('${fixture.root.path}/ws/sources').deleteSync(recursive: true);
      desk.replaceExam(idleDesk);
      await tester.pumpWidget(const SizedBox.shrink());
      StartedSittings.forgetLoaded();
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await openDeck(tester);
      expect(
        [heldFiles(fixture.root).length, find.text('Take exam').evaluate().length, find.byIcon(Icons.more_vert).evaluate().length],
        [1, 1, 1],
        reason: 'sources removed, exam held: one file, Take exam, one menu',
      );
      await tester.tap(find.byIcon(Icons.more_vert));
      await tester.pumpAndSettle();
      expect(
        find.text('Discard exam'),
        findsOneWidget,
        reason: 'the menu of the held exam offers Discard exam',
      );
    },
  );

  testWidgets(
    'a claim that writes the file and then meets an expired pairing still announces the exam',
    (tester) async {
      final fixture = await examFixture('exam-claim-expired-after-write');
      writeExamDeck(fixture.root, 'other.md', 'Other exam');
      final desk = FakeServerClient(
        versionReply: minServerVersion,
        idleBeforeStart: true,
        examStartSitting: corpusSitting(),
        examGetReplies: [deskState('generating', corpusSitting())],
        examCarriedReply: contract('CarriedReplyDto'),
        expireOnExamClose: true,
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await openDeck(tester);
      await tapGenerate(tester);
      await closeSheet(tester);
      desk.replaceExam(deskState('ready', corpusSitting()));

      await openDeck(tester, 'Other exam');
      await tapGenerate(tester);
      await tester.pump();
      expect(
        [
          heldFiles(fixture.root).length,
          desk.examStartCalls,
          find.text('Exam ready: Remote exam.').evaluate().length,
        ],
        [1, 1, 1],
        reason: 'step 1: the claim wrote the file, its close expired: no second start, one announcement',
      );
      await closeSheet(tester);
      await openDeck(tester);
      expect(
        find.text('Take exam'),
        findsOneWidget,
        reason: 'step 2: the deck whose exam the claim wrote offers Take exam',
      );
    },
  );

  testWidgets(
    'a claim that writes the file and then fails to save its record still announces the exam',
    (tester) async {
      final fixture = await examFixture('exam-claim-unsaved-after-write');
      writeExamDeck(fixture.root, 'other.md', 'Other exam');
      final desk = FakeServerClient(
        versionReply: minServerVersion,
        idleBeforeStart: true,
        examStartSitting: corpusSitting(),
        examGetReplies: [deskState('generating', corpusSitting())],
        examCarriedReply: contract('CarriedReplyDto'),
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await openDeck(tester);
      await tapGenerate(tester);
      await closeSheet(tester);
      blockRecordSaves(fixture.support);
      desk.replaceExam(deskState('ready', corpusSitting()));

      await openDeck(tester, 'Other exam');
      await tapGenerate(tester);
      await tester.pump();
      expect(
        [
          heldFiles(fixture.root).length,
          desk.examStartCalls,
          find.text('Exam ready: Remote exam.').evaluate().length,
        ],
        [1, 1, 1],
        reason: 'step 1: the claim wrote the file, its record save threw: no second start, one announcement',
      );
      await closeSheet(tester);
      await openDeck(tester);
      expect(
        find.text('Take exam'),
        findsOneWidget,
        reason: 'step 2: the deck whose exam the claim wrote offers Take exam',
      );
    },
  );

  testWidgets(
    'a result left with an outcome is not announced by the next hand-off even when it was never announced',
    (tester) async {
      final fixture = await examFixture('exam-leave-unannounced');
      final desk = fetchingDesk();
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await generateAndFetch(tester);
      final file = File('${fixture.support.path}/exam-sittings.json');
      file.writeAsStringSync(
        file.readAsStringSync().replaceAll('"announced":true', '"announced":false'),
      );
      StartedSittings.forgetLoaded();
      desk.replaceExam(idleDesk);
      await tester.pumpWidget(const SizedBox.shrink());
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      storeResult(fixture.deck, fixture.root);
      await openDeck(tester);
      await tester.tap(find.text('Show result'));
      await tester.pumpAndSettle();
      await leaveExam(tester);
      expect(
        [heldFiles(fixture.root).length, find.text('Exam ready: Remote exam.').evaluate().length],
        [0, 0],
        reason: 'step 1: the result was left, the file is gone, nothing is announced',
      );

      desk.replaceExam(deskState('ready', corpusSitting()));
      final closesBefore = desk.examClosedSittings.length;
      await tester.pumpWidget(const SizedBox.shrink());
      StartedSittings.forgetLoaded();
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await tester.pump();
      expect(
        [desk.examClosedSittings.skip(closesBefore).toList(), find.text('Exam ready: Remote exam.').evaluate().length],
        [
          [corpusSitting()],
          0,
        ],
        reason: 'step 2: the restarted poll handed the ready exam off (one close) and announced nothing',
      );
    },
  );

  testWidgets(
    'the discard confirm of a held exam shows the deck title it has now, not the one it had when the exam started',
    (tester) async {
      final fixture = await examFixture('exam-discard-renamed');
      final desk = fetchingDesk();
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await generateAndFetch(tester);
      final deck = File(fixture.deck);
      deck.writeAsStringSync(
        deck.readAsStringSync().replaceFirst('title: Remote exam', 'title: Renamed exam'),
      );
      desk.replaceExam(idleDesk);
      await tester.pumpWidget(const SizedBox.shrink());
      StartedSittings.forgetLoaded();
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);

      await openDeck(tester, 'Renamed exam');
      await chooseDiscard(tester);
      expect(
        [
          find.descendant(of: find.byType(AlertDialog), matching: find.text('Renamed exam')).evaluate().length,
          find.descendant(of: find.byType(AlertDialog), matching: find.text('Remote exam')).evaluate().length,
        ],
        [1, 0],
        reason: 'renamed from Remote exam to Renamed exam: the confirm names Renamed exam only',
      );
    },
  );

  testWidgets(
    'a failed generation this phone did not start is not closed, and the start is still sent',
    (tester) async {
      final fixture = await examFixture('exam-failed-not-ours');
      final desk = FakeServerClient(
        versionReply: minServerVersion,
        examStartSitting: corpusSitting(),
        examGetReplies: const [
          RemoteExam(
            sitting: 'a-sitting-of-another-device',
            phase: 'generating',
            deck: 'ws/exam.md',
            thinking: false,
            error: 'the generation failed',
          ),
        ],
      );
      await pumpExamPicker(tester, root: fixture.root, support: fixture.support, deskFor: (_) => desk);
      await openDeck(tester);
      await tapGenerate(tester);
      expect(
        [desk.examClosedSittings, desk.examStartCalls],
        [<String>[], 1],
        reason: 'another device failed generation: no close, one start',
      );
    },
  );
}

class _FakeAccess implements PlatformAccess {
  const _FakeAccess();

  @override
  Future<String?> appVersion() async => '0.2.0+3';
}

class _FailFirstExamCloseClient extends FakeServerClient {
  _FailFirstExamCloseClient({required String sitting})
    : super(
        versionReply: minServerVersion,
        idleBeforeStart: true,
        examStartSitting: sitting,
      );

  var _failed = false;

  @override
  Future<bool> examClose(String sitting) async {
    examCloseCalls++;
    if (!_failed) {
      _failed = true;
      throw StateError('transient exam close failure');
    }
    examClosedSittings.add(sitting);
    return true;
  }
}

class _DeskHandle implements ServerClient {
  _DeskHandle(this.desk);

  final FakeServerClient desk;
  bool closed = false;
  int examGets = 0;

  @override
  Future<ServerVersion?> version() => desk.version();

  @override
  Future<String?> backendName() => desk.backendName();

  @override
  Future<bool> postAsk(TutorCardContext card, List<TutorTurn> history, String question) =>
      desk.postAsk(card, history, question);

  @override
  Future<RemoteAsk?> getAsk() => desk.getAsk();

  @override
  Future<bool> postDraft(TutorCardContext card, List<TutorTurn> history) =>
      desk.postDraft(card, history);

  @override
  Future<bool> postNote(TutorCardContext card, List<TutorTurn> history) =>
      desk.postNote(card, history);

  @override
  Future<AddCardReply> postCard(String deckId, String cardId, String front, List<String> back) =>
      desk.postCard(deckId, cardId, front, back);

  @override
  Future<RemoteExam?> examStart(String deck, {List<String>? cards, Map<String, dynamic>? ideas}) =>
      desk.examStart(deck, cards: cards, ideas: ideas);

  @override
  Future<RemoteExam?> examGet() {
    examGets++;
    return desk.examGet();
  }

  @override
  Future<String?> examCarried(String sitting) => desk.examCarried(sitting);

  @override
  Future<bool> examClose(String sitting) => desk.examClose(sitting);

  @override
  Future<GradingCall> gradingSubmit(String requestJson) => desk.gradingSubmit(requestJson);

  @override
  Future<GradingCall> gradingRemediate(String requestJson) => desk.gradingRemediate(requestJson);

  @override
  Future<GradingCall> gradingGet() => desk.gradingGet();

  @override
  Future<bool> gradingClose(String sitting) => desk.gradingClose(sitting);

  @override
  Future<bool> generateStart(String url, {String? guidance}) =>
      desk.generateStart(url, guidance: guidance);

  @override
  Future<RemoteGenerate?> generateGet() => desk.generateGet();

  @override
  Future<void> generateClose() => desk.generateClose();

  @override
  void close() => closed = true;
}
