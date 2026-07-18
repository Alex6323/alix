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
import 'package:alix_mobile/src/rust/api/simple.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

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

  @override
  Future<String?> appVersion() async => '9.9.9+9';
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
  },
      skip: 'A pick now needs cached AI distractors (offline sampling removed '
          '2026-07-18, CHANGELOG Breaking). Re-enable in the mobile-0.2 build '
          "by seeding this fixture's augment sidecar; the lockstep invariant is "
          'still covered by the rust choose_agrees_with_the_served_options.');

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

  testWidgets('About shows the app and the embedded core versions',
      (tester) async {
    final root = makeRoot();
    addTearDown(() => root.deleteSync(recursive: true));
    await tester.pumpWidget(AlixApp(
      prepared: Prepared(root: root.path, device: 'phone-test'),
      access: FakeAccess(),
      persistDecksDir: (_) async {},
      reprepare: () async => Prepared(root: root.path, device: 'phone-test'),
    ));
    await tester.pumpAndSettle();
    await tester.tap(find.byType(PopupMenuButton<String>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('About'));
    await tester.pumpAndSettle();
    expect(
      find.text('mobile 9.9.9+9 / core ${coreVersion()}'),
      findsOneWidget,
    );
  });

  testWidgets('About carries the one quiet Support line, and only About',
      (tester) async {
    final root = makeRoot();
    addTearDown(() => root.deleteSync(recursive: true));
    await tester.pumpWidget(AlixApp(
      prepared: Prepared(root: root.path, device: 'phone-test'),
      access: FakeAccess(),
      persistDecksDir: (_) async {},
      reprepare: () async => Prepared(root: root.path, device: 'phone-test'),
    ));
    await tester.pumpAndSettle();

    // Not on the picker (a study surface) before About is opened.
    expect(find.textContaining('sponsors/Alex6323'), findsNothing);

    await tester.tap(find.byType(PopupMenuButton<String>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('About'));
    await tester.pumpAndSettle();

    expect(
      find.textContaining('Telling someone who studies is the best support'),
      findsOneWidget,
    );
    expect(
      find.text('https://github.com/sponsors/Alex6323'),
      findsOneWidget,
    );
  });

  group('theme picker (T6.2)', () {
    testWidgets('startup resolves the saved theme via themeById',
        (tester) async {
      final root = makeRoot();
      addTearDown(() => root.deleteSync(recursive: true));
      await tester.pumpWidget(AlixApp(
        prepared:
            Prepared(root: root.path, device: 'phone-test', themeId: 'dracula'),
      ));
      await tester.pumpAndSettle();

      final theme = Theme.of(tester.element(find.byType(PickerScreen)));
      expect(theme.colorScheme.surface, themeById('dracula').colorScheme.surface);
    });

    testWidgets('no saved theme resolves to the dark default', (tester) async {
      final root = makeRoot();
      addTearDown(() => root.deleteSync(recursive: true));
      await tester.pumpWidget(AlixApp(
        prepared: Prepared(root: root.path, device: 'phone-test'),
      ));
      await tester.pumpAndSettle();

      final theme = Theme.of(tester.element(find.byType(PickerScreen)));
      expect(theme.colorScheme.surface, alixDark().colorScheme.surface);
    });

    testWidgets('an unknown saved theme id falls back to dark, no crash',
        (tester) async {
      final root = makeRoot();
      addTearDown(() => root.deleteSync(recursive: true));
      await tester.pumpWidget(AlixApp(
        prepared: Prepared(
          root: root.path,
          device: 'phone-test',
          themeId: 'not-a-real-theme',
        ),
      ));
      await tester.pumpAndSettle();

      expect(tester.takeException(), isNull);
      final theme = Theme.of(tester.element(find.byType(PickerScreen)));
      expect(theme.colorScheme.surface, alixDark().colorScheme.surface);
    });

    testWidgets(
        'the theme sheet lists themes grouped Dark/Light with the current '
        'one marked, and tapping a theme re-themes the app live and persists',
        (tester) async {
      final support = Directory.systemTemp.createTempSync('alix-support-');
      addTearDown(() => support.deleteSync(recursive: true));
      final root = makeRoot();
      addTearDown(() => root.deleteSync(recursive: true));

      await tester.pumpWidget(AlixApp(
        prepared: Prepared(root: root.path, device: 'phone-test'),
        persistTheme: (id) => setTheme(id, support: support),
      ));
      await tester.pumpAndSettle();

      await tester.tap(find.byType(PopupMenuButton<String>));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Theme…'));
      await tester.pumpAndSettle();

      // Grouped under the two mode headers.
      expect(find.text('DARK'), findsOneWidget);
      final scrollable = find.descendant(
        of: find.byKey(const ValueKey('theme-sheet-list')),
        matching: find.byType(Scrollable),
      );
      await tester.scrollUntilVisible(find.text('LIGHT'), 300,
          scrollable: scrollable);
      expect(find.text('LIGHT'), findsOneWidget);

      // No saved theme yet: the dark default carries the one current-marker.
      await tester.scrollUntilVisible(
        find.byKey(const ValueKey('theme-tile-dark')),
        -300,
        scrollable: scrollable,
      );
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('theme-tile-dark')),
          matching: find.byIcon(Icons.check),
        ),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('theme-tile-dracula')),
          matching: find.byIcon(Icons.check),
        ),
        findsNothing,
      );

      await tester.tap(find.byKey(const ValueKey('theme-tile-dracula')));
      await tester.pumpAndSettle();

      // The sheet closed; the whole app re-themed live, with no restart.
      expect(find.byKey(const ValueKey('theme-sheet-list')), findsNothing);
      final theme = Theme.of(tester.element(find.byType(PickerScreen)));
      expect(theme.colorScheme.surface, themeById('dracula').colorScheme.surface);

      // Persisted via the injected seam.
      expect(readTheme(support), 'dracula');
    });
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

    // The rubric renders as tickable keypoint rows; the verdict chip mirrors
    // the tally. Nothing ticked reads as a fail.
    expect(find.byKey(const ValueKey('kp-0')), findsOneWidget);
    expect(find.byKey(const ValueKey('kp-1')), findsOneWidget);
    expect(find.text('Failed'), findsOneWidget);

    // Tick both keypoints: the verdict chip flips to pass.
    await tester.tap(find.byKey(const ValueKey('kp-0')));
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('kp-1')));
    await tester.pump();
    expect(find.text('Passed'), findsOneWidget);

    // The verdict chip commits the tick-derived grade. The store's review
    // history records the grade itself, so a full tally MUST land as a Pass;
    // the done screen alone can't tell (a Fail also floors the card).
    await tester.tap(find.text('Passed'));
    await tester.pump();
    expect(find.text('SESSION COMPLETE'), findsOneWidget);
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

    // The grade trio carries the web's labels, each tinted from the tokens.
    final tokens = Theme.of(tester.element(find.text('Got it'))).alix;
    Color? chipColor(String label) =>
        tester.widget<Text>(find.text(label)).style?.color;
    expect(chipColor('Missed it'), tokens.again);
    expect(chipColor('Partly'), tokens.warn);
    expect(chipColor('Got it'), tokens.good);

    await tester.tap(find.text('Got it'));
    await tester.pump();
    expect(find.text('SESSION COMPLETE'), findsOneWidget);
    expect(
      File('${root.path}/progress.json').readAsStringSync(),
      contains('"stability"'),
    );
  });

  testWidgets('a deck too small for a choice quiz still offers a way forward',
      (tester) async {
    // OBSOLETE as of 2026-07-18: Recognize is now pick-only (assemble filters
    // the roster to recognizable cards), so an un-augmented deck yields an EMPTY
    // Recognize session, not a reveal fallback — the picker greys Recognize out
    // instead. Rework in the mobile-0.2 build (the picker should not offer
    // Recognize here, and this should assert the empty/greyed path).
    final root = makeRoot();
    addTearDown(() => root.deleteSync(recursive: true));
    final deck = '${root.path}/loose.txt';
    final backdated =
        BigInt.from(DateTime.now().millisecondsSinceEpoch - 600000);
    final s =
        ReviewSession.open(deckPath: deck, rootDir: root.path, nowMs: backdated);
    s.acquire(nowMs: backdated);

    await tester.pumpWidget(MaterialApp(
      theme: alixDark(),
      home: ReviewScreen(
        deckPath: deck,
        rootDir: root.path,
        depth: Depth.recognize,
      ),
    ));
    await tester.pumpAndSettle();
    expect(find.text('capital of france?'), findsOneWidget);
    // The way forward: reveal the answer, then grade it. A recognize card
    // with no pick grades with the web's Knew it / Not yet chips.
    expect(find.text('Reveal'), findsOneWidget,
        reason: 'a card with no choices must still be answerable');
    await tester.tap(find.text('Reveal'));
    await tester.pump();
    expect(find.text('Paris'), findsOneWidget);
    await tester.tap(find.text('Knew it'));
    await tester.pump();
    expect(find.text('SESSION COMPLETE'), findsOneWidget);
  }, skip: true); // Recognize is pick-only now; rework in the mobile-0.2 build.

  testWidgets('a choice pick washes the correct option green',
      (tester) async {
    final root = makeRoot();
    addTearDown(() => root.deleteSync(recursive: true));
    final deck = '${root.path}/ws/m.txt';
    acquireAll(deck, root.path);

    await tester.pumpWidget(MaterialApp(
      theme: alixDark(),
      home: ReviewScreen(
        deckPath: deck,
        rootDir: root.path,
        depth: Depth.recognize,
      ),
    ));
    await tester.pumpAndSettle();
    final tokens = Theme.of(tester.element(find.byType(ReviewScreen))).alix;
    // Options render as bordered rows keyed by index; pick the first.
    expect(find.byKey(const ValueKey('option-0')), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('option-0')));
    await tester.pump();

    // After a pick the correct option tints green (its number and text),
    // and the pick locks (the row is no longer a tappable Material/InkWell).
    final greens = tester
        .widgetList<Text>(find.byType(Text))
        .where((t) => t.style?.color == tokens.good)
        .length;
    expect(greens, greaterThanOrEqualTo(1),
        reason: 'the correct option washes green');
    expect(
      find.descendant(
          of: find.byKey(const ValueKey('option-0')),
          matching: find.byType(InkWell)),
      findsNothing,
      reason: 'the options lock after a pick',
    );
    // Renders a real pick, which now needs cached AI distractors (offline
    // sampling removed 2026-07-18, CHANGELOG Breaking). Re-enable in the
    // mobile-0.2 build by seeding this fixture's augment sidecar.
  }, skip: true);
}
