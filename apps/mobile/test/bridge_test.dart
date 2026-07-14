// Tier-1 mobile tests: the Dart-visible bridge API and the screens, driven
// against the REAL embedded core (frb's default loader picks up the host
// dylib from rust/target/release/; `make mobile-test` builds it first).
// Time is injected through the bridge's nowMs, so nothing here sleeps.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/api/listing.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';

/// Acquired at T0, quizzed once the cooldown has elapsed.
final t0 = BigInt.from(1000000);
final later = BigInt.from(1000000 + 61000);

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

  testWidgets('review flows from reveal to grade on a due card',
      (tester) async {
    final root = makeRoot();
    addTearDown(() => root.deleteSync(recursive: true));
    final deck = '${root.path}/loose.txt';
    // Backdate the acquire far enough that the real clock is past the
    // cooldown: the UI (which always uses the wall clock) then serves the
    // first quiz immediately.
    final backdated =
        BigInt.from(DateTime.now().millisecondsSinceEpoch - 120000);
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
