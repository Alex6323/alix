import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/sync/sync_controller.dart';
import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync/sync_sheet.dart';
import 'package:alix_mobile/sync_client.dart';
import 'package:alix_mobile/trace_screen.dart';

import 'support/deck_fixture.dart';
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
    return (root: root, workspace: workspace, support: support);
  }

  Future<_MidSessionConflictController> openSession(
    WidgetTester tester, {
    required String deckId,
    required String deckPath,
    required String rowTitle,
  }) async {
    final fixture = workspace();
    final syncController = _MidSessionConflictController(
      deckId: deckId,
      deckPath: deckPath,
    );
    addTearDown(syncController.dispose);
    await tester.pumpWidget(
      MaterialApp(
        home: PickerScreen(
          root: fixture.root.path,
          dir: fixture.workspace.path,
          title: 'Ws',
          supportDir: fixture.support,
          syncController: syncController,
          isPairedSubtree: true,
          isPushedRoute: true,
        ),
      ),
    );
    await settlePicker(tester);
    await tester.tap(find.text(rowTitle));
    await tester.pumpAndSettle();
    if (rowTitle == 'Deck') {
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
}

class _MidSessionConflictController extends ChangeNotifier
    implements SyncController {
  _MidSessionConflictController({required this.deckId, required this.deckPath});

  final String deckId;
  final String deckPath;
  bool conflicted = false;
  int resolved = 0;

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
    conflicted = false;
    notifyListeners();
  }

  @override
  Future<void> removeOrphan(String entry) async {}
}
