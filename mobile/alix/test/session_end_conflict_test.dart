import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/sync/sync_controller.dart';
import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync/sync_sheet.dart';
import 'package:alix_mobile/sync_client.dart';
import 'package:alix_mobile/trace_screen.dart';

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
      '---\ntitle: Deck\nsource: source.txt\n---\n## q\na\n',
    );
    File(
      '${workspace.path}/decks/source.txt',
    ).writeAsStringSync('alpha\nbeta\n');
    writeTestDeck(
      '${workspace.path}/decks/trace.md',
      '---\n'
          'trace: a paired trace\n'
          'source: source.txt\n'
          'title: Trace\n'
          '---\n'
          '## Predict\n'
          'it reads line one\n'
          '<!-- at: 1 -->\n',
    );
    writeTestDeck(
      '${workspace.path}/decks/two.md',
      '---\ntitle: Two\n---\n## q1\na1\n\n## q2\na2\n',
    );
    writeTestDeck(
      '${workspace.path}/decks/trace2.md',
      '---\n'
          'trace: a two-hop paired trace\n'
          'source: source.txt\n'
          'title: Trace2\n'
          '---\n'
          '## Predict one\n'
          'it reads line one\n'
          '<!-- at: 1 -->\n'
          '\n'
          '## Predict two\n'
          'it reads line two\n'
          '<!-- at: 2 -->\n',
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
    bool directTrace = false,
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
    if (directTrace) {
      await tester.pumpWidget(
        MaterialApp(
          home: TraceSessionScreen(
            deckPath: '${fixture.workspace.path}/$deckPath',
            rootDir: fixture.root.path,
            supportDir: fixture.support,
            buildClient: (_) => examClient!,
            syncController: syncController,
          ),
        ),
      );
      await tester.pumpAndSettle();
    } else {
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
    await tester.tap(find.text("Take the desktop's"));
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

  testWidgets(
    'passing the exam after taking desktop on a finished review keeps the '
    'desktop card progress',
    (tester) async {
      final client = FakeServerClient(
        versionReply: '0.8.0',
        examGetReplies: [_passedExam(isTrace: false)],
      );
      final syncController = await openSession(
        tester,
        deckId: 'deck-1',
        deckPath: 'decks/deck.md',
        rowTitle: 'Deck',
        examClient: client,
      );
      await tester.tap(find.text('Reveal'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Seen'));
      await tester.pumpAndSettle();
      expect(find.text('SESSION COMPLETE'), findsOneWidget);

      await resolveAndExpectPicker(tester, syncController);
      expect(_cards(syncController.workspace), isEmpty);
      await tester.tap(find.text('Take the exam'));
      await tester.pumpAndSettle();

      expect(find.text('Passed.'), findsOneWidget);
      expect(
        _cards(syncController.workspace),
        isEmpty,
        reason: 'exam mastery must not restore the discarded phone reviews',
      );
    },
  );

  testWidgets(
    'passing the exam after taking desktop on a finished trace keeps the '
    'desktop card progress',
    (tester) async {
      final client = FakeServerClient(
        versionReply: '0.8.0',
        examGetReplies: [_passedExam(isTrace: true)],
      );
      final syncController = await openSession(
        tester,
        deckId: 'trace-1',
        deckPath: 'decks/trace.md',
        rowTitle: 'Trace',
        examClient: client,
        directTrace: true,
      );
      await tester.enterText(find.byType(TextField), 'a guess');
      await tester.tap(find.text('Reveal'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Got it'));
      await tester.pumpAndSettle();
      expect(find.text('TRACE COMPLETE'), findsOneWidget);

      await resolveAndExpectPicker(tester, syncController);
      expect(_cards(syncController.workspace), isEmpty);
      await tester.tap(find.text('Take the exam'));
      await tester.pumpAndSettle();

      expect(find.text('Passed.'), findsOneWidget);
      expect(
        _cards(syncController.workspace),
        isEmpty,
        reason: 'exam mastery must not restore the discarded phone trace',
      );
    },
  );
}

RemoteExam _passedExam({required bool isTrace}) => RemoteExam(
  phase: 'results',
  deck: 'ws/deck.md',
  strictness: 'balanced',
  questions: const [],
  passed: true,
  grades: const [],
  gaps: const [],
  canRemediate: false,
  isTrace: isTrace,
  thinking: false,
);

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
      digest: 'xxh64-0000000000000001',
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
  Future<void> cycle({String? entry}) async {}

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
