// Tier-1 mobile tests: the Dart-visible bridge API and the screens, driven
// against the REAL embedded core (frb's default loader picks up the host
// dylib from rust/target/release/; `make mobile-test` builds it first).
// Time is injected through the bridge's nowMs, so nothing here sleeps.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/main.dart';
import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/platform_access.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/api/listing.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';

/// The platform seam's test double: no channels exist under `flutter test`.
class FakeAccess implements PlatformAccess {
  FakeAccess({this.dir});

  /// What the "picker" returns; null models a cancel.
  final String? dir;

  @override
  Future<bool> supportsSharedFolders() async => true;

  @override
  Future<bool> hasAllFilesAccess() async => true;

  @override
  Future<bool> ensureAllFilesAccess() async => true;

  @override
  Future<String?> pickDirectory() async => dir;
}

/// Acquired at T0, quizzed once the cooldown has elapsed. 301000 = the
/// core's DEFAULT_ACQUIRE_COOLDOWN_MS (5 min, src/scheduler.rs) + 1s; keep
/// them in step.
final t0 = BigInt.from(1000000);
final later = BigInt.from(1000000 + 301000);

/// A decks root with one loose deck and one workspace member deck.
Directory makeRoot() {
  final root = Directory.systemTemp.createTempSync('alix-decks-');
  File('${root.path}/loose.txt').writeAsStringSync(
    '% title: Loose\n# capital of france?\n    Paris\n',
  );
  Directory('${root.path}/ws').createSync();
  File('${root.path}/ws/alix.toml').writeAsStringSync('title = "Ws"\n');
  File('${root.path}/ws/m.txt').writeAsStringSync(
    '# q1\n    a1\n# q2\n    a2\n# q3\n    a3\n# q4\n    a4\n',
  );
  return root;
}

/// Acquires every card of a deck at T0 through the real bridge, so a session
/// opened at `later` serves the first quiz. The store lands wherever the
/// core routes it.
void acquireAll(String deck, String root) {
  final s = ReviewSession.open(deckPath: deck, rootDir: root, nowMs: t0);
  var state = s.state();
  while (state.acquire) {
    state = s.acquire(nowMs: t0);
  }
}

void main() {
  setUpAll(() async => RustLib.init());

  test('listing sees the workspace and the loose deck', () {
    final root = makeRoot();
    addTearDown(() => root.deleteSync(recursive: true));
    final rows = listRoot(root: root.path, nowMs: t0);
    expect(
      rows.map((r) => (r.title, r.isWorkspace, r.due)).toList(),
      [('Loose', false, true), ('Ws', true, true)],
    );
    final members =
        listMembers(root: root.path, dir: '${root.path}/ws', nowMs: t0);
    expect(members.single.title, 'm');
  });

  test('a grade persists into the workspace store, on injected time', () {
    final root = makeRoot();
    addTearDown(() => root.deleteSync(recursive: true));
    final deck = '${root.path}/ws/m.txt';
    acquireAll(deck, root.path);

    final s = ReviewSession.open(
      deckPath: deck,
      rootDir: root.path,
      nowMs: later,
    );
    expect(s.state().acquire, isFalse);
    expect(s.state().mode, Mode.flip);
    s.grade(grade: Grade.pass, nowMs: later);
    final store = File('${root.path}/ws/progress.json').readAsStringSync();
    expect(store, contains('"stability"'));
    final rootStore = File('${root.path}/progress.json');
    expect(
      !rootStore.existsSync() ||
          !rootStore.readAsStringSync().contains('"stability"'),
      isTrue,
      reason: 'the loose-deck root store stays untouched (or was never made)',
    );
  });

  test('choice options and feedback stay in lockstep', () {
    final root = makeRoot();
    addTearDown(() => root.deleteSync(recursive: true));
    final deck = '${root.path}/ws/m.txt';
    acquireAll(deck, root.path);

    final s = ReviewSession.open(
      deckPath: deck,
      rootDir: root.path,
      depth: Depth.recognize,
      nowMs: later,
    );
    final options = s.state().choices;
    expect(options, isNotNull);
    expect(options!.length, 4);
    final correct = s.choose(chosen: 0)!.correct;
    expect(s.choose(chosen: correct.toInt())!.passed, isTrue);
    expect(s.state().choices, options, reason: 'options hold still');
  });

  testWidgets('the picker lists and drills into the workspace',
      (tester) async {
    final root = makeRoot();
    addTearDown(() => root.deleteSync(recursive: true));
    await tester.pumpWidget(MaterialApp(home: PickerScreen(root: root.path)));
    expect(find.text('Loose'), findsOneWidget);
    await tester.tap(find.text('Ws'));
    await tester.pumpAndSettle();
    expect(find.text('m'), findsOneWidget);
  });

  testWidgets('choosing a shared folder swaps the picker root live',
      (tester) async {
    final support = Directory.systemTemp.createTempSync('alix-support-');
    addTearDown(() => support.deleteSync(recursive: true));
    final rootA = makeRoot();
    addTearDown(() => rootA.deleteSync(recursive: true));
    final rootB = Directory.systemTemp.createTempSync('alix-shared-');
    addTearDown(() => rootB.deleteSync(recursive: true));
    File('${rootB.path}/shared.txt')
        .writeAsStringSync('% title: Shared Deck\n# q\n    a\n');

    await tester.pumpWidget(AlixApp(
      prepared: Prepared(root: rootA.path, device: 'phone-test'),
      access: FakeAccess(dir: rootB.path),
      persistDecksDir: (dir) => setDecksDir(dir, support: support),
      reprepare: () => prepare(support: support, env: ''),
    ));
    await tester.pumpAndSettle();
    expect(find.text('Loose'), findsOneWidget);

    await tester.tap(find.byType(PopupMenuButton<String>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Decks folder…'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Choose shared folder…'));
    await tester.pumpAndSettle();

    expect(find.text('Shared Deck'), findsOneWidget);
    expect(find.text('Loose'), findsNothing);
  });

  testWidgets('a cancelled folder pick leaves the root unchanged',
      (tester) async {
    final support = Directory.systemTemp.createTempSync('alix-support-');
    addTearDown(() => support.deleteSync(recursive: true));
    final root = makeRoot();
    addTearDown(() => root.deleteSync(recursive: true));

    await tester.pumpWidget(AlixApp(
      prepared: Prepared(root: root.path, device: 'phone-test'),
      access: FakeAccess(dir: null),
      persistDecksDir: (dir) => setDecksDir(dir, support: support),
      reprepare: () => prepare(support: support, env: ''),
    ));
    await tester.pumpAndSettle();
    await tester.tap(find.byType(PopupMenuButton<String>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Decks folder…'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Choose shared folder…'));
    await tester.pumpAndSettle();

    expect(find.text('Loose'), findsOneWidget);
    expect(find.textContaining('stays on its current'), findsOneWidget);
  });

  testWidgets('the picker warns about a sync conflict file until dismissed',
      (tester) async {
    final root = makeRoot();
    addTearDown(() => root.deleteSync(recursive: true));
    File('${root.path}/progress.sync-conflict-20260714-101112-AAAAAAA.json')
        .writeAsStringSync('{}');

    await tester.pumpWidget(MaterialApp(home: PickerScreen(root: root.path)));
    await tester.pumpAndSettle();
    expect(find.textContaining('sync conflict'), findsOneWidget);
    await tester.tap(find.byIcon(Icons.close));
    await tester.pump();
    expect(find.textContaining('sync conflict'), findsNothing);
  });

  testWidgets('the review screen warns when another device wrote the store',
      (tester) async {
    final root = makeRoot();
    addTearDown(() => root.deleteSync(recursive: true));
    final deck = '${root.path}/loose.txt';
    final backdated =
        BigInt.from(DateTime.now().millisecondsSinceEpoch - 600000);
    final s = ReviewSession.open(
      deckPath: deck,
      rootDir: root.path,
      nowMs: backdated,
      device: 'desk-1',
    );
    s.acquire(nowMs: backdated);

    await tester.pumpWidget(MaterialApp(
      home: ReviewScreen(
        deckPath: deck,
        rootDir: root.path,
        depth: Depth.recall,
        device: 'phone-1',
      ),
    ));
    await tester.pumpAndSettle();
    expect(find.textContaining("Last written by 'desk-1'"), findsOneWidget);
    await tester.tap(find.byIcon(Icons.close));
    await tester.pump();
    expect(find.textContaining('Last written by'), findsNothing);

    // The store's last writer is now this screen's own device (opening
    // saves), so a re-open as the same device stays quiet.
    await tester.pumpWidget(MaterialApp(
      home: ReviewScreen(
        deckPath: deck,
        rootDir: root.path,
        depth: Depth.recall,
        device: 'phone-1',
      ),
    ));
    await tester.pumpAndSettle();
    expect(find.textContaining('Last written by'), findsNothing);
  });

  test('keypointGrade maps the tally like core', () {
    expect(keypointGrade(covered: 0, total: 3), Grade.fail);
    expect(keypointGrade(covered: 2, total: 3), Grade.partial);
    expect(keypointGrade(covered: 3, total: 3), Grade.pass);
    expect(keypointGrade(covered: 0, total: 0), Grade.pass,
        reason: 'no rubric, nothing to miss');
  });

  testWidgets('the explain checklist derives the grade from the ticks',
      (tester) async {
    final root = makeRoot();
    addTearDown(() => root.deleteSync(recursive: true));
    // A seen multi-line flip card at Reconstruct renders as Explain; with no
    // cached keypoints the rubric falls back to the authored back lines.
    final deck = '${root.path}/why.txt';
    File(deck).writeAsStringSync(
      '# why does spacing work?\n'
      '    recall strengthens the memory\n'
      '    stronger memories fade more slowly\n',
    );
    final backdated =
        BigInt.from(DateTime.now().millisecondsSinceEpoch - 600000);
    final s = ReviewSession.open(
        deckPath: deck, rootDir: root.path, nowMs: backdated);
    s.acquire(nowMs: backdated);

    await tester.pumpWidget(MaterialApp(
      home: ReviewScreen(
        deckPath: deck,
        rootDir: root.path,
        depth: Depth.reconstruct,
      ),
    ));
    expect(find.text('why does spacing work?'), findsOneWidget);
    await tester.tap(find.text('Reveal'));
    await tester.pump();

    // The rubric renders as tickable rows; nothing ticked reads as a fail.
    expect(find.byType(CheckboxListTile), findsNWidgets(2));
    expect(find.textContaining('0/2'), findsOneWidget);

    // Tick both keypoints: the live verdict flips to pass.
    await tester.tap(find.byType(CheckboxListTile).at(0));
    await tester.pump();
    await tester.tap(find.byType(CheckboxListTile).at(1));
    await tester.pump();
    expect(find.textContaining('2/2'), findsOneWidget);

    // Continue commits the tick-derived grade. The store's review history
    // records the grade itself, so a full tally MUST land as a Pass; a
    // "Done for now" screen alone can't tell (a Fail also floors the card).
    await tester.tap(find.text('Continue'));
    await tester.pump();
    expect(find.text('Done for now'), findsOneWidget);
    final store = File('${root.path}/progress.json').readAsStringSync();
    expect(store, contains('"reconstruct"'));
    expect(store, contains('"Pass"'),
        reason: 'all keypoints ticked grades as a Pass, not a Fail');
  });

  testWidgets('review flows from reveal to grade on a due card',
      (tester) async {
    final root = makeRoot();
    addTearDown(() => root.deleteSync(recursive: true));
    final deck = '${root.path}/loose.txt';
    // Backdate the acquire far enough that the real clock is past the
    // cooldown (5 min default): the UI (which always uses the wall clock)
    // then serves the first quiz immediately.
    final backdated =
        BigInt.from(DateTime.now().millisecondsSinceEpoch - 600000);
    final s =
        ReviewSession.open(deckPath: deck, rootDir: root.path, nowMs: backdated);
    s.acquire(nowMs: backdated);

    await tester.pumpWidget(MaterialApp(
      home: ReviewScreen(
        deckPath: deck,
        rootDir: root.path,
        depth: Depth.recall,
      ),
    ));
    expect(find.text('capital of france?'), findsOneWidget);
    await tester.tap(find.text('Reveal'));
    await tester.pump();
    expect(find.text('Paris'), findsOneWidget);
    await tester.tap(find.text('Pass'));
    await tester.pump();
    expect(find.text('Done for now'), findsOneWidget);
    expect(
      File('${root.path}/progress.json').readAsStringSync(),
      contains('"stability"'),
    );
  });
}
