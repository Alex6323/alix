import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/crumb_strip.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

import 'support/deck_fixture.dart';
import 'support/widget_tree_dump.dart';

void main() {
  setUpAll(() async => RustLib.init());

  Directory tempDir(String prefix) {
    final dir = Directory.systemTemp.createTempSync(prefix);
    addTearDown(() {
      if (dir.existsSync()) {
        if (Platform.isLinux || Platform.isMacOS) {
          Process.runSync('chmod', ['-R', 'u+rwx', dir.path]);
        }
        dir.deleteSync(recursive: true);
      }
    });
    return dir;
  }

  Directory support() => tempDir('alix-review-structure-support-');

  Future<void> pumpReview(
    WidgetTester tester, {
    required Directory root,
    required String deck,
    required ReviewDepth depth,
    String? device,
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: ReviewScreen(
          key: UniqueKey(),
          deckPath: deck,
          rootDir: root.path,
          depth: depth,
          device: device,
          supportDir: support(),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  void makeDue(String deck, String root, {String? device}) {
    final then = BigInt.from(
      DateTime.now().millisecondsSinceEpoch -
          const Duration(minutes: 10).inMilliseconds,
    );
    final session = ReviewSession.open(
      deckPath: deck,
      rootDir: root,
      nowMs: then,
      device: device,
    );
    while (session.state(nowMs: then).introducing) {
      session.introduce(nowMs: then);
    }
  }

  testWidgets('review tree: cannot-open and fresh-introduce surfaces', (
    tester,
  ) async {
    final root = tempDir('alix-review-structure-open-');
    final trace = '${root.path}/trace.md';
    File('${root.path}/source.txt').writeAsStringSync('one\ntwo\n');
    writeTestDeck(
      trace,
      '---\ntrace: path\nsource: source.txt\n---\n'
      '## Predict\nanswer\n<!-- at: 1 -->\n',
    );
    await pumpReview(
      tester,
      root: root,
      deck: trace,
      depth: ReviewDepth.recall,
    );
    await expectWidgetTree(
      tester,
      'review_cannot_open',
      root: find.byType(ReviewScreen),
    );

    final fresh = '${root.path}/fresh.md';
    writeTestDeck(fresh, '## Fresh question?\nFresh answer\n');
    await pumpReview(
      tester,
      root: root,
      deck: fresh,
      depth: ReviewDepth.recall,
    );
    await expectWidgetTree(
      tester,
      'review_introduce_hidden',
      root: find.byType(ReviewScreen),
    );
    await tester.tap(find.text('Reveal'));
    await tester.pump();
    await expectWidgetTree(
      tester,
      'review_introduce_revealed',
      root: find.byType(ReviewScreen),
    );
  });

  testWidgets(
    'review tree: flip before reveal, after reveal, and with both banners',
    (tester) async {
      final root = tempDir('alix-review-structure-flip-');
      final deck = '${root.path}/flip.md';
      writeTestDeck(
        deck,
        '## First question? <!-- id: card-first -->\nFirst answer\n\n'
        '## Second question? <!-- id: card-second -->\nSecond answer\n',
      );
      makeDue(deck, root.path, device: 'desktop');
      await pumpReview(
        tester,
        root: root,
        deck: deck,
        depth: ReviewDepth.recall,
        device: 'phone',
      );
      await expectWidgetTree(
        tester,
        'review_flip_hidden',
        root: find.byType(ReviewScreen),
      );
      await tester.tap(find.text('Reveal'));
      await tester.pump();
      await expectWidgetTree(
        tester,
        'review_flip_revealed',
        root: find.byType(ReviewScreen),
      );

      final progress = Directory('${root.path}/progress');
      final savedProgress = Directory('${root.path}/progress.saved');
      progress.renameSync(savedProgress.path);
      File(progress.path).writeAsStringSync('blocks the progress directory');
      await tester.tap(find.text('Got it'));
      await tester.pump();
      File(progress.path).deleteSync();
      savedProgress.renameSync(progress.path);
      expect(find.textContaining("Progress isn't being saved"), findsOneWidget);
      expect(find.textContaining("Last written by 'desktop'"), findsOneWidget);
      await expectWidgetTree(
        tester,
        'review_flip_save_and_foreign_banners',
        root: find.byType(ReviewScreen),
      );
    },
  );

  testWidgets('review tree: choice before and after a pick', (tester) async {
    final root = tempDir('alix-review-structure-choice-');
    final deck = '${root.path}/choice.md';
    writeTestDeck(
      deck,
      '## Capital of France? <!-- id: card-choice -->\n'
      '- [x] Paris\n- [ ] Rome\n- [ ] Bern\n- [ ] Madrid\n',
    );
    makeDue(deck, root.path);
    await pumpReview(
      tester,
      root: root,
      deck: deck,
      depth: ReviewDepth.recognize,
    );
    await expectWidgetTree(
      tester,
      'review_choice_waiting',
      root: find.byType(ReviewScreen),
    );
    await tester.tap(find.text('Paris'));
    await tester.pump();
    await expectWidgetTree(
      tester,
      'review_choice_answered',
      root: find.byType(ReviewScreen),
    );
  });

  testWidgets('review tree: typing and type-line before and after check', (
    tester,
  ) async {
    final root = tempDir('alix-review-structure-typing-');
    final typing = '${root.path}/typing.md';
    writeTestDeck(typing, '## Atomic? <!-- id: card-atomic -->\nanswer\n');
    makeDue(typing, root.path);
    await pumpReview(
      tester,
      root: root,
      deck: typing,
      depth: ReviewDepth.reconstruct,
    );
    await expectWidgetTree(
      tester,
      'review_typing_waiting',
      root: find.byType(ReviewScreen),
    );
    await tester.enterText(find.byType(TextField), 'answer');
    await tester.tap(find.text('Submit'));
    await tester.pump();
    await expectWidgetTree(
      tester,
      'review_typing_checked',
      root: find.byType(ReviewScreen),
    );

    final typeLine = '${root.path}/type-line.md';
    writeTestDeck(
      typeLine,
      '## Ordered? <!-- id: card-ordered -->\n'
      '<!-- reveal: line -->\n'
      'first\nsecond\n',
    );
    makeDue(typeLine, root.path);
    await pumpReview(
      tester,
      root: root,
      deck: typeLine,
      depth: ReviewDepth.reconstruct,
    );
    await expectWidgetTree(
      tester,
      'review_type_line_waiting',
      root: find.byType(ReviewScreen),
    );
    final fields = find.byType(TextField);
    await tester.enterText(fields.at(0), 'first');
    await tester.enterText(fields.at(1), 'wrong');
    await tester.tap(find.text('Check'));
    await tester.pump();
    await expectWidgetTree(
      tester,
      'review_type_line_checked',
      root: find.byType(ReviewScreen),
    );
  });

  testWidgets('review tree: line-by-line hidden, partial, and complete', (
    tester,
  ) async {
    final root = tempDir('alix-review-structure-lines-');
    final deck = '${root.path}/lines.md';
    writeTestDeck(
      deck,
      '## Ordered? <!-- id: card-lines -->\n'
      '<!-- reveal: line -->\n'
      'first\nsecond\n',
    );
    makeDue(deck, root.path);
    await pumpReview(tester, root: root, deck: deck, depth: ReviewDepth.recall);
    await expectWidgetTree(
      tester,
      'review_line_by_line_hidden',
      root: find.byType(ReviewScreen),
    );
    await tester.tap(find.text('Reveal'));
    await tester.pump();
    await expectWidgetTree(
      tester,
      'review_line_by_line_partial',
      root: find.byType(ReviewScreen),
    );
    await tester.tap(find.text('Reveal next'));
    await tester.pump();
    await expectWidgetTree(
      tester,
      'review_line_by_line_complete',
      root: find.byType(ReviewScreen),
    );
  });

  testWidgets('review tree: explain attempt, checklist, and verdict', (
    tester,
  ) async {
    final root = tempDir('alix-review-structure-explain-');
    final deck = '${root.path}/explain.md';
    writeTestDeck(
      deck,
      '## Why? <!-- id: card-explain -->\nfirst reason\nsecond reason\n',
    );
    makeDue(deck, root.path);
    await pumpReview(
      tester,
      root: root,
      deck: deck,
      depth: ReviewDepth.reconstruct,
    );
    await expectWidgetTree(
      tester,
      'review_explain_closed_attempt',
      root: find.byType(ReviewScreen),
    );
    await tester.tap(find.text('type your answer first'));
    await tester.pump();
    await tester.enterText(find.byType(TextField), 'my reason');
    await expectWidgetTree(
      tester,
      'review_explain_open_attempt',
      root: find.byType(ReviewScreen),
    );
    await tester.tap(find.text('Reveal'));
    await tester.pump();
    await expectWidgetTree(
      tester,
      'review_explain_checklist',
      root: find.byType(ReviewScreen),
    );
    await tester.tap(find.byKey(const ValueKey('kp-0')));
    await tester.pump();
    await expectWidgetTree(
      tester,
      'review_explain_partial_verdict',
      root: find.byType(ReviewScreen),
    );
  });

  testWidgets('review tree: topology crumb and drained summaries', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: Scaffold(
          body: CrumbStrip(
            crumb: ReviewCrumbModel(
              regions: const ['Intro', 'Details'],
              current: 1,
              cells: const [
                ['learned-strong', 'learned-fading'],
                ['seen', 'learned-weak'],
              ],
            ),
          ),
        ),
      ),
    );
    await expectWidgetTree(
      tester,
      'review_topology_crumb',
      root: find.byType(CrumbStrip),
    );

    final root = tempDir('alix-review-structure-summary-');
    final deck = '${root.path}/summary.md';
    writeTestDeck(deck, '## New? <!-- id: card-new -->\nanswer\n');
    await pumpReview(tester, root: root, deck: deck, depth: ReviewDepth.recall);
    await tester.tap(find.text('Reveal'));
    await tester.pump();
    await tester.tap(find.text('Seen'));
    await tester.pump();
    await expectWidgetTree(
      tester,
      'review_summary_introduced',
      root: find.byType(ReviewScreen),
    );

    await pumpReview(tester, root: root, deck: deck, depth: ReviewDepth.recall);
    await expectWidgetTree(
      tester,
      'review_summary_nothing_due',
      root: find.byType(ReviewScreen),
    );
  });
}
