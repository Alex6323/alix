import 'dart:async';
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
import 'support/fake_sync_port.dart';
import 'support/picker_listing.dart';

void main() {
  setUpAll(() async => RustLib.init());

  ({Directory root, Directory workspace, Directory support}) factWorkspace() {
    final root = Directory.systemTemp.createTempSync('alix-gate-yield-root-');
    final support = Directory.systemTemp.createTempSync(
      'alix-gate-yield-support-',
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
    return (root: root, workspace: workspace, support: support);
  }

  Future<void> pumpWorkspace(
    WidgetTester tester, {
    required Directory root,
    required Directory workspace,
    required Directory support,
    required SyncController syncController,
    List<NavigatorObserver> observers = const [],
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        navigatorObservers: observers,
        home: PickerScreen(
          root: root.path,
          dir: workspace.path,
          title: 'Ws',
          supportDir: support,
          syncController: syncController,
          isPairedSubtree: true,
          isPushedRoute: true,
        ),
      ),
    );
    await settlePicker(tester);
  }

  testWidgets(
    'the last conflict read and the session push run with no yield between '
    'them',
    (tester) async {
      final fixture = factWorkspace();
      final pages = _PageRouteCounter();
      final syncController = _YieldingConflictController(fixture.root.path);
      syncController.onPublish = () => pages.pushed;
      addTearDown(syncController.dispose);
      await pumpWorkspace(
        tester,
        root: fixture.root,
        workspace: fixture.workspace,
        support: fixture.support,
        syncController: syncController,
        observers: [pages],
      );

      await tester.tap(find.text('Deck'));
      await tester.pumpAndSettle();
      expect(find.text('Recall'), findsOneWidget);
      expect(syncController.pendingReads, 1);
      final pagesBefore = pages.pushed;

      await tester.tap(find.text('Recall'));
      await tester.pump();

      expect(
        syncController.pagesPushedAtPublish,
        pagesBefore + 1,
        reason:
            'a conflict published right after the last read must find '
            'the review route already pushed',
      );
      await tester.pumpAndSettle();
    },
  );

  testWidgets('a conflicted paired trace opens the conflict choice before its '
      'progress-writing trace session', (tester) async {
    final fixture = factWorkspace();
    File(
      '${fixture.workspace.path}/decks/source.txt',
    ).writeAsStringSync('alpha\nbeta\n');
    File('${fixture.workspace.path}/decks/deck.md').deleteSync();
    writeTestDeck(
      '${fixture.workspace.path}/decks/trace.md',
      '---\n'
          'trace: a paired trace\n'
          'source: source.txt\n'
          'title: Trace\n'
          '---\n'
          '## Predict\n'
          'it reads line one\n'
          '<!-- at: 1 -->\n',
    );
    final port = FakeSyncPort(rootDir: fixture.root.path);
    port.pairedEntriesImpl = () => const [
      SyncEntryState(
        entry: 'ws',
        kind: 'workspace',
        digest: 'xxh64-0000000000000001',
        decks: [
          SyncDeckState(
            deckId: 'trace-1',
            path: 'decks/trace.md',
            unpushed: true,
            conflict: PairedConflictPush(desktopRevision: 2),
          ),
        ],
      ),
    ];
    final syncController = SyncController(port: port);
    addTearDown(syncController.dispose);
    await pumpWorkspace(
      tester,
      root: fixture.root,
      workspace: fixture.workspace,
      support: fixture.support,
      syncController: syncController,
    );

    await tester.tap(find.text('Trace'));
    await tester.pumpAndSettle();

    expect(find.byType(TraceSessionScreen), findsNothing);
    expect(find.byType(SyncReportSheet), findsOneWidget);
  });

  testWidgets(
    'resolving a conflict found by the second gate returns to the picker',
    (tester) async {
      final fixture = factWorkspace();
      var conflicted = false;
      final port = FakeSyncPort(rootDir: fixture.root.path);
      port.entriesImpl = () async => const SyncEntries(
        rootId: 'root-test0000000000000000000000',
        entries: [
          SyncEntry(
            name: 'ws',
            kind: 'workspace',
            members: 1,
            unpackedBytes: 10,
            digest: 'xxh64-0000000000000001',
            leftOut: [],
          ),
        ],
      );
      port.pairedEntriesImpl = () => [
        SyncEntryState(
          entry: 'ws',
          kind: 'workspace',
          digest: 'xxh64-0000000000000001',
          decks: [
            SyncDeckState(
              deckId: 'deck-1',
              path: 'decks/deck.md',
              unpushed: false,
              conflict: conflicted
                  ? const PairedConflictPush(desktopRevision: 2)
                  : null,
            ),
          ],
        ),
      ];
      port.resolveConflictImpl = (_, _) {
        conflicted = false;
        return const SyncResolutionDone();
      };
      final syncController = SyncController(port: port);
      addTearDown(syncController.dispose);
      await pumpWorkspace(
        tester,
        root: fixture.root,
        workspace: fixture.workspace,
        support: fixture.support,
        syncController: syncController,
      );

      await tester.tap(find.text('Deck'));
      await tester.pumpAndSettle();
      conflicted = true;
      await syncController.cycle();
      expect(syncController.pendingConflicts, hasLength(1));

      await tester.tap(find.text('Recall'));
      await tester.pumpAndSettle();
      expect(find.byType(SyncReportSheet), findsOneWidget);
      await tester.tap(find.text("Take the desktop's"));
      await tester.pumpAndSettle();

      expect(syncController.pendingConflicts, isEmpty);
      expect(find.byType(SyncReportSheet), findsNothing);
      expect(find.byType(ReviewScreen), findsNothing);
      expect(find.text('Deck'), findsOneWidget);
    },
  );
}

class _YieldingConflictController extends ChangeNotifier
    implements SyncController {
  _YieldingConflictController(this.rootDir);

  final String rootDir;
  int pendingReads = 0;
  bool conflicted = false;
  int Function()? onPublish;
  int? pagesPushedAtPublish;

  @override
  bool get running => false;

  @override
  SyncReport? get lastReport => null;

  @override
  bool get reportUnread => false;

  @override
  String? get statusLine => null;

  @override
  List<SyncEntryState> get pairedEntries => const [
    SyncEntryState(
      entry: 'ws',
      kind: 'workspace',
      digest: 'xxh64-0000000000000001',
      decks: [
        SyncDeckState(deckId: 'deck-1', path: 'decks/deck.md', unpushed: false),
      ],
    ),
  ];

  @override
  List<SyncPendingConflict> get pendingConflicts {
    pendingReads++;
    if (pendingReads == 2) {
      scheduleMicrotask(() {
        pagesPushedAtPublish = onPublish?.call();
        conflicted = true;
        notifyListeners();
      });
    }
    return conflicted
        ? const [
            SyncPendingConflict(
              deckId: 'deck-1',
              label: 'Ws/deck.md',
              conflict: PairedConflictPush(desktopRevision: 2),
            ),
          ]
        : const [];
  }

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
    conflicted = false;
    notifyListeners();
  }

  @override
  Future<void> removeOrphan(String entry) async {}
}

class _PageRouteCounter extends NavigatorObserver {
  int pushed = 0;

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) {
    if (route is MaterialPageRoute) pushed++;
  }
}
