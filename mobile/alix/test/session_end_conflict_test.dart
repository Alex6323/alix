import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/bridge/exam_bridge.dart' as exam_bridge;
import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/api/exam.dart' as bridge;
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/sync/sync_controller.dart';
import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync/sync_sheet.dart';
import 'package:alix_mobile/sync_client.dart';
import 'package:alix_mobile/trace_screen.dart';

import 'support/carried_exam_fixture.dart';
import 'support/deck_fixture.dart';
import 'support/fake_server_client.dart';
import 'support/picker_listing.dart';

void main() {
  setUpAll(() async => RustLib.init());

  ({Directory root, Directory workspace, Directory support}) workspace() {
    final root = Directory.systemTemp.createTempSync('alix-session-end-root-');
    final support = Directory.systemTemp.createTempSync(
      'alix-session-end-support-',
    );
    addTearDown(() {
      if (root.existsSync()) root.deleteSync(recursive: true);
      if (support.existsSync()) support.deleteSync(recursive: true);
    });
    final workspace = Directory('${root.path}/ws');
    Directory('${workspace.path}/decks').createSync(recursive: true);
    File('${workspace.path}/alix.toml').writeAsStringSync('title = "Ws"\n');
    writeTestDeck(
      '${workspace.path}/decks/deck.md',
      '---\ntitle: Deck\n---\n## q\na\n',
    );
    writeTestSources('${workspace.path}/decks/deck.md');
    File(
      '${workspace.path}/decks/source.txt',
    ).writeAsStringSync('alpha\nbeta\n');
    writeTestDeck(
      '${workspace.path}/decks/trace.md',
      '---\n'
          'trace: a paired trace\n'
          'cite:\n'
          '  - src: decks\n'
          'title: Trace\n'
          '---\n'
          '## Predict\n'
          'it reads line one\n'
          '<!-- at: src:source.txt:1 -->\n',
    );
    writeTestDeck(
      '${workspace.path}/decks/two.md',
      '---\ntitle: Two\n---\n## q1\na1\n\n## q2\na2\n',
    );
    writeTestDeck(
      '${workspace.path}/decks/trace2.md',
      '---\n'
          'trace: a two-hop paired trace\n'
          'cite:\n'
          '  - src: decks\n'
          'title: Trace2\n'
          '---\n'
          '## Predict one\n'
          'it reads line one\n'
          '<!-- at: src:source.txt:1 -->\n'
          '\n'
          '## Predict two\n'
          'it reads line two\n'
          '<!-- at: src:source.txt:2 -->\n',
    );
    return (root: root, workspace: workspace, support: support);
  }

  Future<_MidSessionConflictController> openSession(
    WidgetTester tester, {
    required String deckId,
    required String deckPath,
    required String rowTitle,
    bool review = false,
    FakeServerClient? examClient,
  }) async {
    final fixture = workspace();
    if (examClient != null) {
      await savePairing(
        const ServerConfig(
          host: '127.0.0.1',
          port: 7777,
          token: 'session-end-test',
          rootId: 'root-test0000000000000000000000',
        ),
        support: fixture.support,
      );
    }
    final syncController = _MidSessionConflictController(
      deckId: deckId,
      deckPath: deckPath,
      workspace: fixture.workspace,
    );
    addTearDown(syncController.dispose);
    await tester.pumpWidget(
      MaterialApp(
        home: PickerScreen(
          root: fixture.root.path,
          dir: fixture.workspace.path,
          title: 'Ws',
          supportDir: fixture.support,
          buildClient: examClient == null ? null : (_) => examClient,
          syncController: syncController,
          isPairedSubtree: true,
          isPushedRoute: true,
        ),
      ),
    );
    await settlePicker(tester);
    await tester.tap(find.text(rowTitle));
    await tester.pumpAndSettle();
    if (rowTitle == 'Deck' || review) {
      await tester.tap(find.text('Recall'));
      await tester.pumpAndSettle();
    }
    syncController.publish();
    await tester.pumpAndSettle();
    expect(
      find.byType(SyncReportSheet),
      findsNothing,
      reason: 'a conflict published mid-session must not interrupt it',
    );
    return syncController;
  }

  Future<void> leaveEarly(WidgetTester tester) async {
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    await tester.tap(find.text('Leave'));
    await tester.pumpAndSettle();
  }

  Future<void> resolveAndExpectPicker(
    WidgetTester tester,
    _MidSessionConflictController syncController,
  ) async {
    await tester.tap(find.text("Keep the desktop's progress"));
    await tester.pumpAndSettle();
    expect(syncController.resolved, 1, reason: 'the choice reaches resolve');
    expect(find.byType(SyncReportSheet), findsNothing);
  }

  testWidgets(
    'a conflict published mid-review opens its choice when the review '
    'finishes',
    (tester) async {
      final syncController = await openSession(
        tester,
        deckId: 'deck-1',
        deckPath: 'decks/deck.md',
        rowTitle: 'Deck',
      );

      await tester.tap(find.text('Reveal'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Seen'));
      await tester.pumpAndSettle();

      expect(find.text('SESSION COMPLETE'), findsOneWidget);
      expect(find.byType(SyncReportSheet), findsOneWidget);
      await resolveAndExpectPicker(tester, syncController);
    },
  );

  testWidgets(
    'a conflict published mid-review opens its choice when the learner '
    'leaves early',
    (tester) async {
      final syncController = await openSession(
        tester,
        deckId: 'deck-1',
        deckPath: 'decks/deck.md',
        rowTitle: 'Deck',
      );

      await leaveEarly(tester);

      expect(find.byType(SyncReportSheet), findsOneWidget);
      await resolveAndExpectPicker(tester, syncController);
      expect(find.byType(ReviewScreen), findsNothing);
    },
  );

  testWidgets(
    'a conflict published mid-trace opens its choice when the trace finishes',
    (tester) async {
      final syncController = await openSession(
        tester,
        deckId: 'trace-1',
        deckPath: 'decks/trace.md',
        rowTitle: 'Trace',
      );

      await tester.enterText(find.byType(TextField), 'a guess');
      await tester.tap(find.text('Reveal'));
      await tester.pumpAndSettle();
      expect(find.byType(SyncReportSheet), findsNothing);
      await tester.tap(find.text('Got it'));
      await tester.pumpAndSettle();

      expect(find.text('TRACE COMPLETE'), findsOneWidget);
      expect(find.byType(SyncReportSheet), findsOneWidget);
      await resolveAndExpectPicker(tester, syncController);
    },
  );

  testWidgets(
    'a conflict published mid-trace opens its choice when the learner '
    'leaves early',
    (tester) async {
      final syncController = await openSession(
        tester,
        deckId: 'trace-1',
        deckPath: 'decks/trace.md',
        rowTitle: 'Trace',
      );

      await leaveEarly(tester);

      expect(find.byType(SyncReportSheet), findsOneWidget);
      await resolveAndExpectPicker(tester, syncController);
      expect(find.byType(TraceSessionScreen), findsNothing);
    },
  );

  Future<void> expectDesktopKept(
    WidgetTester tester,
    _MidSessionConflictController syncController,
    String step,
  ) async {
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(find.byType(ReviewScreen), findsNothing, reason: step);
    expect(find.byType(TraceSessionScreen), findsNothing, reason: step);
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pumpAndSettle();
    final desktop = syncController.pulledDesktop;
    expect(desktop, isNotNull, reason: '$step: the desktop side was pulled');
    expect(
      progressDocument(syncController.workspace).readAsStringSync(),
      desktop,
      reason:
          '$step: the closed session must not write its stale progress '
          'over the pulled desktop document',
    );
  }

  testWidgets(
    'taking the desktop side after leaving a review early keeps the pulled '
    'progress once the review closes',
    (tester) async {
      final syncController = await openSession(
        tester,
        deckId: 'two-1',
        deckPath: 'decks/two.md',
        rowTitle: 'Two',
        review: true,
      );
      await tester.tap(find.text('Reveal'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Seen'));
      await tester.pumpAndSettle();
      expect(find.text('SESSION COMPLETE'), findsNothing);

      await leaveEarly(tester);
      await resolveAndExpectPicker(tester, syncController);
      await expectDesktopKept(tester, syncController, 'review left early');
    },
  );

  testWidgets(
    'taking the desktop side on a finished review summary keeps the pulled '
    'progress once the review closes',
    (tester) async {
      final syncController = await openSession(
        tester,
        deckId: 'deck-1',
        deckPath: 'decks/deck.md',
        rowTitle: 'Deck',
      );
      await tester.tap(find.text('Reveal'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Seen'));
      await tester.pumpAndSettle();
      expect(find.text('SESSION COMPLETE'), findsOneWidget);

      await resolveAndExpectPicker(tester, syncController);
      await expectDesktopKept(tester, syncController, 'review finished');
    },
  );

  testWidgets(
    'taking the desktop side after leaving a trace early keeps the pulled '
    'progress once the trace closes',
    (tester) async {
      final syncController = await openSession(
        tester,
        deckId: 'trace2-1',
        deckPath: 'decks/trace2.md',
        rowTitle: 'Trace2',
      );
      await tester.enterText(find.byType(TextField), 'a guess');
      await tester.tap(find.text('Reveal'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Got it'));
      await tester.pumpAndSettle();
      expect(find.text('TRACE COMPLETE'), findsNothing);

      await leaveEarly(tester);
      await resolveAndExpectPicker(tester, syncController);
      await expectDesktopKept(tester, syncController, 'trace left early');
    },
  );

  testWidgets(
    'taking the desktop side on a finished trace keeps the pulled progress '
    'once the trace closes',
    (tester) async {
      final syncController = await openSession(
        tester,
        deckId: 'trace-1',
        deckPath: 'decks/trace.md',
        rowTitle: 'Trace',
      );
      await tester.enterText(find.byType(TextField), 'a guess');
      await tester.tap(find.text('Reveal'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Got it'));
      await tester.pumpAndSettle();
      expect(find.text('TRACE COMPLETE'), findsOneWidget);

      await resolveAndExpectPicker(tester, syncController);
      await expectDesktopKept(tester, syncController, 'trace finished');
    },
  );

  Future<
    ({
      _MidSessionConflictController syncController,
      exam_bridge.BridgeHeldExam held,
    })
  >
  takeDesktopThenHoldGradedExam(
    WidgetTester tester, {
    FakeServerClient? examClient,
  }) async {
    final syncController = await openSession(
      tester,
      deckId: 'deck-1',
      deckPath: 'decks/deck.md',
      rowTitle: 'Deck',
      examClient: examClient,
    );
    await tester.tap(find.text('Reveal'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Seen'));
    await tester.pumpAndSettle();
    expect(find.text('SESSION COMPLETE'), findsOneWidget);

    await resolveAndExpectPicker(tester, syncController);
    final desktop = syncController.pulledDesktop;
    expect(
      [
        progressDocument(syncController.workspace).readAsStringSync(),
        _cards(syncController.workspace),
      ],
      [desktop, isEmpty],
      reason:
          'after the resolve: the progress document is the pulled desktop '
          'one, with no card progress',
    );
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();

    final deck = '${syncController.workspace.path}/decks/deck.md';
    final root = syncController.workspace.parent.path;
    writeVerifiedTestSources(deck);
    final start = bridge.examSittingStart(
      deckPath: deck,
      rootDir: root,
      needsWorkOnly: false,
      nowMs: BigInt.from(DateTime.now().millisecondsSinceEpoch),
    );
    bridge.heldExamReceive(
      rootDir: root,
      carriedJson: matchedExamFor(deck),
      start: start,
    );
    final held = exam_bridge.BridgeHeldExam(
      deckPath: deck,
      deckId: start.deckId,
      rootDir: root,
    );
    held.storeResult(contract('GradingReply.results'));

    await tester.tap(find.text('Deck'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Show result'));
    await tester.pumpAndSettle();
    expect(
      find.textContaining('This exam is stale'),
      findsNothing,
      reason:
          'opening the held result: a stale line means the fixture no '
          'longer matches the carried exam',
    );
    return (syncController: syncController, held: held);
  }

  void expectRecorded(
    _MidSessionConflictController syncController,
    exam_bridge.BridgeHeldExam held,
    String step,
  ) {
    final recorded = held.view()?.outcome?.recorded;
    final progress = jsonDecode(
      progressDocument(syncController.workspace).readAsStringSync(),
    ) as Map;
    final fullResult = (progress['deck'] as Map)['full_exam_result'];
    expect(
      [recorded, fullResult],
      [
        true,
        {'at_ms': isA<int>(), 'pass': 0, 'partial': 0, 'total': 1},
      ],
      reason:
          '$step: the held outcome is recorded ($recorded) and the progress '
          "document holds the sitting's failed full result ($fullResult)",
    );
  }

  testWidgets(
    'a held exam result applied after taking the desktop side on a finished '
    'review is recorded and brings no discarded review back',
    (tester) async {
      final (:syncController, :held) = await takeDesktopThenHoldGradedExam(
        tester,
      );

      expectRecorded(syncController, held, 'applied');
      expect(
        _cards(syncController.workspace),
        isEmpty,
        reason:
            'applied: the card progress is still the desktop one, so no '
            'discarded phone review came back',
      );
    },
  );

  testWidgets(
    'remediating a held exam after taking the desktop side on a finished '
    'review lands the card in the sidecar and brings no discarded review back',
    (tester) async {
      final remediated = contract('GradingReply.remediated');
      final client = FakeServerClient(
        versionReply: minServerVersion,
        gradingReplies: [
          GradingAnswered(
            remediated,
            RemoteGrading.fromJson(
              jsonDecode(remediated) as Map<String, dynamic>,
            )!,
          ),
        ],
      );
      final (:syncController, :held) = await takeDesktopThenHoldGradedExam(
        tester,
        examClient: client,
      );
      expectRecorded(syncController, held, 'before remediating');

      await tester.tap(find.text('Turn the gaps into cards'));
      await tester.pumpAndSettle();

      final sidecar = File(
        '${syncController.workspace.path}/decks/deck.local.md',
      );
      expect(
        sidecar.existsSync() ? sidecar.readAsStringSync() : '<no sidecar>',
        contains('## Why does Rust use ownership?'),
        reason:
            'remediated: the remediation card lives in the sidecar, '
            'remediate requests ${client.remediateRequests.length}',
      );
      expect(
        _cards(syncController.workspace),
        isEmpty,
        reason:
            'remediated: the card progress is still the desktop one, so no '
            'discarded phone review came back',
      );
    },
  );
}

Map<String, dynamic> _cards(Directory workspace) {
  final json = jsonDecode(progressDocument(workspace).readAsStringSync());
  return (json as Map<String, dynamic>)['cards'] as Map<String, dynamic>;
}

File progressDocument(Directory workspace) {
  final documents = Directory('${workspace.path}/.alix/progress')
      .listSync()
      .whereType<File>()
      .where((f) => f.path.endsWith('.json'))
      .where((f) => !f.uri.pathSegments.last.startsWith('recent'))
      .toList();
  expect(documents, hasLength(1), reason: 'one deck progress document');
  return documents.single;
}

String desktopVersionOf(String phoneDocument) {
  final json = jsonDecode(phoneDocument) as Map<String, dynamic>;
  json['cards'] = <String, dynamic>{};
  return jsonEncode(json);
}

class _MidSessionConflictController extends ChangeNotifier
    implements SyncController {
  _MidSessionConflictController({
    required this.deckId,
    required this.deckPath,
    required this.workspace,
  });

  final String deckId;
  final String deckPath;
  final Directory workspace;
  bool conflicted = false;
  int resolved = 0;
  String? pulledDesktop;

  void publish() {
    conflicted = true;
    notifyListeners();
  }

  @override
  bool get running => false;

  @override
  SyncReport? get lastReport => null;

  @override
  bool get reportUnread => false;

  @override
  String? get statusLine => null;

  @override
  List<SyncEntryState> get pairedEntries => [
    SyncEntryState(
      entry: 'ws',
      kind: 'workspace',
      fingerprint: '0000000000001',
      decks: [
        SyncDeckState(
          deckId: deckId,
          path: deckPath,
          unpushed: false,
          conflict: conflicted
              ? const PairedConflictPush(desktopRevision: 2)
              : null,
        ),
      ],
    ),
  ];

  @override
  List<SyncPendingConflict> get pendingConflicts => conflicted
      ? [
          SyncPendingConflict(
            deckId: deckId,
            label: 'Ws/$deckPath',
            conflict: const PairedConflictPush(desktopRevision: 2),
          ),
        ]
      : const [];

  @override
  Set<String> get unpushedEntries => const {};

  @override
  List<SyncEntry> get availableEntries => const [];

  @override
  void markReportRead() {}

  @override
  Future<void> cycle({String? entry, bool everyListed = false}) async {}

  @override
  Future<void> pushOne(String deckId) async {}

  @override
  Future<void> resolve(String deckId, {required bool keepPhone}) async {
    resolved++;
    if (!keepPhone &&
        Directory('${workspace.path}/.alix/progress').existsSync()) {
      final document = progressDocument(workspace);
      final desktop = desktopVersionOf(document.readAsStringSync());
      document.writeAsStringSync(desktop);
      pulledDesktop = desktop;
    }
    conflicted = false;
    notifyListeners();
  }

  @override
  Future<void> removeOrphan(String entry) async {}
}
