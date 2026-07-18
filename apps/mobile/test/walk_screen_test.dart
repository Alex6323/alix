// T5.2 widget tests: the on-device trace walk, driven against the REAL
// embedded core (RustLib.init in setUpAll; real deck files on disk),
// mirroring bridge_test.dart's own fixtures/pattern and the review screen's
// exam-chip tests (review_screen_exam_chip_test.dart) for the client-
// acquisition seam. A trace fixture always carries an in-folder `% source:`
// so its checkpoints resolve real gutter lines, matching the rust bridge's
// own `trace_fixture` (apps/mobile/rust/src/api/review.rs).
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/exam_screen.dart';
import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/walk_screen.dart';

import 'support/fake_server_client.dart';

void main() {
  setUpAll(() async => RustLib.init());

  Directory tempRoot(String prefix) {
    final dir = Directory.systemTemp.createTempSync(prefix);
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  Directory tempSupport() {
    final dir = Directory.systemTemp.createTempSync('alix-walk-support-');
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  /// A two-hop trace over a real in-folder source file, matching the rust
  /// bridge's own `trace_fixture` verbatim (apps/mobile/rust/src/api/review.rs).
  Directory twoHopRoot() {
    final root = tempRoot('alix-walk-2hop-');
    File('${root.path}/source.txt').writeAsStringSync('first\nsecond\nthird\n');
    File('${root.path}/t.txt').writeAsStringSync(
      '% trace: how it works\n'
      '% source: source.txt\n'
      '# Predict the first hop\n'
      '\tit reads the first line\n'
      '\t% at: 1\n'
      '# Predict the second hop\n'
      '\tit reads lines two and three\n'
      '\t% at: 2-3\n',
    );
    return root;
  }

  /// A one-hop trace, for tests only interested in reaching the done
  /// screen quickly (the exam-handoff visibility rules).
  Directory oneHopRoot() {
    final root = tempRoot('alix-walk-1hop-');
    File('${root.path}/source.txt').writeAsStringSync('first\nsecond\n');
    File('${root.path}/t.txt').writeAsStringSync(
      '% trace: a short path\n'
      '% source: source.txt\n'
      '# Predict the hop\n'
      '\tit reads the first line\n'
      '\t% at: 1\n',
    );
    return root;
  }

  /// A trace whose checkpoint locator has nothing to resolve against (no
  /// `% source:` at all): the excerpt-error fallback path.
  Directory noSourceRoot() {
    final root = tempRoot('alix-walk-nosource-');
    File('${root.path}/t.txt').writeAsStringSync(
      '% trace: a path with no source\n'
      '# Predict something\n'
      '\tthe answer\n'
      '\t% at: 1\n',
    );
    return root;
  }

  group('walking a trace end-to-end', () {
    testWidgets(
        'predict shows the prompt, reveal shows the real excerpt and points, '
        'grading advances hops, and done shows the tally', (tester) async {
      final root = twoHopRoot();
      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: WalkScreen(
          deckPath: '${root.path}/t.txt',
          rootDir: root.path,
          supportDir: tempSupport(),
        ),
      ));
      await tester.pumpAndSettle();

      // Hop 1: predict.
      expect(find.text('Predict the first hop'), findsOneWidget);
      expect(find.text('checkpoint 1 / 2'), findsOneWidget);
      await tester.enterText(find.byType(TextField), 'guess one');
      await tester.tap(find.text('Reveal'));
      await tester.pumpAndSettle();

      // Hop 1: reveal shows the real gutter excerpt, not a placeholder.
      expect(find.text('guess one'), findsOneWidget);
      expect(find.textContaining('source.txt'), findsOneWidget);
      expect(find.text('1'), findsOneWidget);
      expect(find.text('first'), findsOneWidget);
      expect(find.text('it reads the first line'), findsOneWidget);
      expect(find.text('Missed it'), findsOneWidget);
      expect(find.text('Partly'), findsOneWidget);
      expect(find.text('Got it'), findsOneWidget);

      await tester.tap(find.text('Got it'));
      await tester.pumpAndSettle();

      // Hop 2: predict.
      expect(find.text('Predict the second hop'), findsOneWidget);
      expect(find.text('checkpoint 2 / 2'), findsOneWidget);
      await tester.enterText(find.byType(TextField), 'guess two');
      await tester.tap(find.text('Reveal'));
      await tester.pumpAndSettle();

      expect(find.text('2'), findsOneWidget);
      expect(find.text('second'), findsOneWidget);
      expect(find.text('3'), findsOneWidget);
      expect(find.text('third'), findsOneWidget);

      await tester.tap(find.text('Partly'));
      await tester.pumpAndSettle();

      // Done: the tally.
      expect(find.text('WALK COMPLETE'), findsOneWidget);
      expect(find.text('got it'), findsOneWidget);
      expect(find.text('partly'), findsOneWidget);
      expect(find.text('missed it'), findsOneWidget);
      expect(find.text('Walk again'), findsOneWidget);

      // No hop rail, no live-grade chrome: only the delta chips + the
      // done actions ever appear; a fresh predict/reveal is what advances.
      expect(find.byType(TextField), findsNothing);
    });

    testWidgets('a checkpoint with no resolvable source shows the excerptError calmly',
        (tester) async {
      final root = noSourceRoot();
      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: WalkScreen(
          deckPath: '${root.path}/t.txt',
          rootDir: root.path,
          supportDir: tempSupport(),
        ),
      ));
      await tester.pumpAndSettle();

      await tester.enterText(find.byType(TextField), 'a guess');
      await tester.tap(find.text('Reveal'));
      await tester.pumpAndSettle();

      // No crash; the reveal still renders with the grade chips available.
      expect(tester.takeException(), isNull);
      expect(find.text('Got it'), findsOneWidget);
      // The honest excerpt_error fallback (a line-only locator with no
      // `% source:` to resolve it against), not a silent gap.
      expect(find.textContaining('is not a single file'), findsOneWidget);
    });
  });

  group('the exam handoff on the done screen', () {
    Future<void> walkOneHopToDone(WidgetTester tester) async {
      await tester.enterText(find.byType(TextField), 'a guess');
      await tester.tap(find.text('Reveal'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Got it'));
      await tester.pumpAndSettle();
    }

    testWidgets(
        'paired, a live probe, no cooldown: "Take the exam" is offered and pushes the exam screen',
        (tester) async {
      final root = oneHopRoot();
      final support = tempSupport();
      await setServer(const ServerConfig(host: '127.0.0.1', port: 7777, token: 'tok'), support: support);

      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: WalkScreen(
          deckPath: '${root.path}/t.txt',
          rootDir: root.path,
          supportDir: support,
          buildClient: (_) => FakeServerClient(versionReply: '0.6.0', examStartReply: true),
        ),
      ));
      await tester.pumpAndSettle();
      await walkOneHopToDone(tester);

      expect(find.text('Take the exam'), findsOneWidget);

      await tester.tap(find.text('Take the exam'));
      // Two pumps: one resolves the tap's gesture arena and schedules the
      // Navigator's push, the next actually builds the pushed route.
      await tester.pump();
      await tester.pump();
      expect(find.byType(ExamScreen), findsOneWidget);
    });

    testWidgets(
        'paired but the probe fails (unreachable): "Take the exam" does not exist even '
        'though a config exists, and the walk itself never touched the server to get here',
        (tester) async {
      final root = oneHopRoot();
      final support = tempSupport();
      await setServer(const ServerConfig(host: '127.0.0.1', port: 7777, token: 'tok'), support: support);

      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: WalkScreen(
          deckPath: '${root.path}/t.txt',
          rootDir: root.path,
          supportDir: support,
          buildClient: (_) => FakeServerClient(versionReply: null, examStartReply: true),
        ),
      ));
      await tester.pumpAndSettle();
      await walkOneHopToDone(tester);

      expect(find.text('Take the exam'), findsNothing);
      expect(find.text('Walk again'), findsOneWidget);
    });

    testWidgets(
        'an older paired server: "Take the exam" does not exist', (tester) async {
      final root = oneHopRoot();
      final support = tempSupport();
      await setServer(const ServerConfig(host: '127.0.0.1', port: 7777, token: 'tok'), support: support);

      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: WalkScreen(
          deckPath: '${root.path}/t.txt',
          rootDir: root.path,
          supportDir: support,
          buildClient: (_) => FakeServerClient(versionReply: '0.5.0'),
        ),
      ));
      await tester.pumpAndSettle();
      await walkOneHopToDone(tester);

      expect(find.text('Take the exam'), findsNothing);
    });

    testWidgets(
        'a refused token (401 on the probe): the exact re-pair SnackBar, '
        'and Re-pair opens the pairing sheet, matching review_screen.dart', (tester) async {
      final root = oneHopRoot();
      final support = tempSupport();
      await setServer(const ServerConfig(host: '127.0.0.1', port: 7777, token: 'stale'), support: support);

      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: WalkScreen(
          deckPath: '${root.path}/t.txt',
          rootDir: root.path,
          supportDir: support,
          buildClient: (_) => FakeServerClient(expireOnVersion: true),
        ),
      ));
      await tester.pumpAndSettle();

      // Mirrors review_screen_ask_chip_test.dart's own 401 test: this
      // screen never pops itself on a 401 (unlike exam_screen.dart), so its
      // own context is still alive; the action must open the sheet on it
      // without throwing. Stays on the predict screen (a still-showing
      // SnackBar occupies the same footer area a reveal tap would need,
      // matching the pre-existing review test's own scope).
      expect(
        find.text('Pairing expired. Pair again from the deck list menu.'),
        findsOneWidget,
      );

      await tester.tap(find.text('Re-pair'));
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('pairing-url-field')), findsOneWidget);
      expect(tester.takeException(), isNull);
    });

    testWidgets('unpaired: "Take the exam" does not exist, the walk stays fully offline',
        (tester) async {
      final root = oneHopRoot();
      final support = tempSupport();

      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: WalkScreen(
          deckPath: '${root.path}/t.txt',
          rootDir: root.path,
          supportDir: support,
        ),
      ));
      await tester.pumpAndSettle();
      await walkOneHopToDone(tester);

      expect(find.text('Take the exam'), findsNothing);
      expect(find.textContaining('re-sitting'), findsNothing);
      expect(find.text('Walk again'), findsOneWidget);
    });

    testWidgets('paired but cooling down from a recent failed sitting: a hint, not the button',
        (tester) async {
      final root = oneHopRoot();
      final support = tempSupport();
      await setServer(const ServerConfig(host: '127.0.0.1', port: 7777, token: 'tok'), support: support);

      // Fail the trace exam moments ago (through the bridge directly, like
      // the rust test `exam_cooldown_gates_a_resit_...`): the default
      // cooldown is an hour, so this is still deep inside the window when
      // the widget checks real wall-clock "now" a moment later.
      final justNow = BigInt.from(DateTime.now().millisecondsSinceEpoch - 500);
      WalkSession.open(deckPath: '${root.path}/t.txt', rootDir: root.path)
          .applyExamFailed(nowMs: justNow);

      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: WalkScreen(
          deckPath: '${root.path}/t.txt',
          rootDir: root.path,
          supportDir: support,
          buildClient: (_) => FakeServerClient(versionReply: '0.6.0'),
        ),
      ));
      await tester.pumpAndSettle();
      await walkOneHopToDone(tester);

      expect(find.text('Take the exam'), findsNothing);
      expect(find.textContaining('re-sitting'), findsOneWidget);
      expect(find.text('Walk again'), findsOneWidget);
    });
  });

  group('the picker routes a trace row to the walk', () {
    testWidgets('tapping a trace row opens WalkScreen, not the old refusal snack',
        (tester) async {
      final root = tempRoot('alix-picker-walk-');
      File('${root.path}/source.txt').writeAsStringSync('alpha\nbeta\n');
      File('${root.path}/t.txt').writeAsStringSync(
        '% title: T\n'
        '% trace: a picker-launched walk\n'
        '% source: source.txt\n'
        '# Predict\n'
        '\tit reads line one\n'
        '\t% at: 1\n',
      );

      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: PickerScreen(root: root.path, supportDir: tempSupport()),
      ));
      await tester.pumpAndSettle();

      await tester.tap(find.text('T'));
      await tester.pumpAndSettle();

      expect(find.byType(WalkScreen), findsOneWidget);
      expect(find.text('Predict'), findsOneWidget);
      expect(
        find.text('Trace decks are guided source walks; for now they '
            'live in the web app.'),
        findsNothing,
      );
    });
  });
}
