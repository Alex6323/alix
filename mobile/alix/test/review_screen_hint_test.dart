import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart' show ReviewChip;
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

import 'support/deck_fixture.dart';

void main() {
  setUpAll(() async => RustLib.init());

  Directory deckRoot(String name, String contents) {
    final root = Directory.systemTemp.createTempSync('alix-hints-$name-');
    writeTestDeck('${root.path}/facts.md', contents);
    addTearDown(() {
      if (root.existsSync()) root.deleteSync(recursive: true);
    });
    return root;
  }

  Directory support() {
    final dir = Directory.systemTemp.createTempSync('alix-hints-support-');
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  void makeDue(Directory root) {
    final then = BigInt.from(
      DateTime.now().millisecondsSinceEpoch -
          const Duration(minutes: 10).inMilliseconds,
    );
    final session = ReviewSession.open(
      deckPath: '${root.path}/facts.md',
      rootDir: root.path,
      nowMs: then,
    );
    while (session.state(nowMs: then).introducing) {
      session.introduce(nowMs: then);
    }
  }

  Future<void> pumpReview(
    WidgetTester tester,
    Directory root, {
    ReviewDepth depth = ReviewDepth.recall,
    bool skipIntroduction = false,
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: ReviewScreen(
          key: ValueKey(root.path),
          deckPath: '${root.path}/facts.md',
          rootDir: root.path,
          depth: depth,
          skipIntroduction: skipIntroduction,
          supportDir: support(),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  Finder chip(String label) => find.widgetWithText(InkWell, label);

  Future<void> tapChip(WidgetTester tester, String label) async {
    final target = chip(label);
    expect(target, findsOneWidget, reason: 'the $label chip is offered');
    await tester.ensureVisible(target);
    await tester.tap(target);
    await tester.pumpAndSettle();
  }

  Future<void> closeHelp(WidgetTester tester) async {
    await tester.tap(find.byKey(const ValueKey('section-sheet-close')));
    await tester.pumpAndSettle();
  }

  const twoHints =
      '## q?\n'
      '> [!TIP]\n'
      '> first clue\n\n'
      '> [!TIP]\n'
      '> second clue\n\n'
      '---\n'
      'answer\n';

  testWidgets('hints open on demand and stack oldest first', (
    tester,
  ) async {
    final root = deckRoot('stack', twoHints);
    makeDue(root);
    await pumpReview(tester, root);

    expect(chip('Hint'), findsOneWidget);
    expect(find.text('first clue'), findsNothing);
    await tapChip(tester, 'Hint');

    expect(find.byKey(const ValueKey('hint-sheet-title')), findsOneWidget);
    expect(find.text('first clue'), findsOneWidget);
    expect(find.text('second clue'), findsNothing);
    expect(
      find.byKey(const ValueKey('review-help-mode-selector')),
      findsNothing,
    );
    expect(find.byKey(const ValueKey('next-hint')), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('next-hint')));
    await tester.pumpAndSettle();
    expect(find.text('second clue'), findsOneWidget);
    expect(
      tester.getTopLeft(find.text('first clue')).dy,
      lessThan(tester.getTopLeft(find.text('second clue')).dy),
      reason: 'authored order is retained in the stack',
    );
    expect(find.byKey(const ValueKey('next-hint')), findsNothing);

    await closeHelp(tester);
    await tapChip(tester, 'Hint');
    expect(find.text('first clue'), findsOneWidget);
    expect(find.text('second clue'), findsOneWidget);
    final reveal = tester.widget<ReviewChip>(
      find.widgetWithText(ReviewChip, 'Reveal'),
    );
    reveal.onTap!();
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('hint-sheet-title')), findsNothing);
    expect(chip('Hint'), findsNothing, reason: 'the answer ended the offer');
  });

  testWidgets('context and opened hints share one retained panel', (
    tester,
  ) async {
    final root = deckRoot(
      'shared',
      '# Capitals\nUse the river and country together.\n\n$twoHints',
    );
    makeDue(root);
    await pumpReview(tester, root);
    await tapChip(tester, 'Context');

    expect(find.byKey(const ValueKey('section-sheet-title')), findsOneWidget);
    expect(find.text('Use the river and country together.'), findsOneWidget);
    expect(
      find.byKey(const ValueKey('review-help-mode-selector')),
      findsOneWidget,
    );
    expect(find.text('first clue'), findsNothing);

    await tester.tap(find.byKey(const ValueKey('review-help-hint')));
    await tester.pumpAndSettle();
    expect(find.text('first clue'), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('next-hint')));
    await tester.pumpAndSettle();
    expect(find.text('second clue'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('review-help-context')));
    await tester.pumpAndSettle();
    expect(find.text('Use the river and country together.'), findsOneWidget);
    expect(find.text('first clue'), findsNothing);
    await tester.tap(find.byKey(const ValueKey('review-help-hint')));
    await tester.pumpAndSettle();
    expect(find.text('first clue'), findsOneWidget);
    expect(find.text('second clue'), findsOneWidget);
  });

  testWidgets('the Hint chip follows every answer-production boundary', (
    tester,
  ) async {
    final hintless = deckRoot('hintless', '## q?\nanswer\n');
    makeDue(hintless);
    await pumpReview(tester, hintless);
    expect(
      chip('Hint'),
      findsNothing,
      reason: 'a card without hints offers none',
    );

    final introduction = deckRoot('introduction', twoHints);
    await pumpReview(tester, introduction);
    expect(
      chip('Hint'),
      findsNothing,
      reason: 'introduction shows no hint offer',
    );

    final choice = deckRoot(
      'choice',
      '## capital?\n'
      '> [!TIP]\n'
      '> it is in France\n\n'
      '---\n'
      '- [x] Paris\n'
      '- [ ] Rome\n'
      '<!-- choices: single -->\n',
    );
    makeDue(choice);
    await pumpReview(tester, choice, depth: ReviewDepth.recognize);
    expect(chip('Hint'), findsOneWidget, reason: 'choice is unanswered');
    await tester.tap(find.text('Paris'));
    await tester.pumpAndSettle();
    expect(
      chip('Hint'),
      findsNothing,
      reason: 'a choice pick answers the card',
    );

    final typing = deckRoot('typing', twoHints);
    makeDue(typing);
    await pumpReview(tester, typing, depth: ReviewDepth.reconstruct);
    expect(chip('Hint'), findsOneWidget, reason: 'typing is unchecked');
    await tester.enterText(find.byType(TextField), 'answer');
    await tapChip(tester, 'Submit');
    expect(
      chip('Hint'),
      findsNothing,
      reason: 'a typed check answers the card',
    );

    final lines = deckRoot(
      'lines',
      '## ordered?\n'
      '> [!TIP]\n'
      '> start at the beginning\n\n'
      '---\n'
      'first\n'
      'second\n'
      '<!-- reveal: line -->\n',
    );
    makeDue(lines);
    await pumpReview(tester, lines);
    expect(chip('Hint'), findsOneWidget, reason: 'no line is revealed');
    await tapChip(tester, 'Reveal');
    expect(chip('Hint'), findsOneWidget, reason: 'more answer lines remain');
    await tapChip(tester, 'Reveal next');
    expect(
      chip('Hint'),
      findsNothing,
      reason: 'the last line ends the offer',
    );
  });

  testWidgets('a both-direction card offers hints only on its forward half', (
    tester,
  ) async {
    final root = deckRoot(
      'both',
      '$twoHints<!-- direction: both -->\n',
    );
    makeDue(root);
    await pumpReview(tester, root);

    expect(
      chip('Hint'),
      findsOneWidget,
      reason: 'forward carries the authored hints',
    );
    await tapChip(tester, 'Reveal');
    await tapChip(tester, 'Got it');
    expect(
      chip('Hint'),
      findsNothing,
      reason: 'the generated reverse has no hints',
    );
  });
}
