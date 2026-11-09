// The topic affordance on the phone: a card with a topic carries no
// heading on its face; a tap on the question opens the topic in a bottom
// sheet, and the sitting's first introduction from a topic opens that
// sheet by itself, once. A topic-less card has no tap target. No pairing
// and no server are involved: the topic rides in the deck. ReviewScreen
// calls the real bridge, so RustLib.init() is required.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart' show TopicSheet;
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

import 'support/deck_fixture.dart';

const _heading = 'Vehicle safety';
const _prose = 'Applies on public roads.';

void main() {
  setUpAll(() async => RustLib.init());

  Directory tempSupport() {
    final dir = Directory.systemTemp.createTempSync('alix-topic-support-');
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  Directory deckRoot(String deck) {
    final root = Directory.systemTemp.createTempSync('alix-topic-decks-');
    writeTestDeck('${root.path}/facts.md', deck);
    addTearDown(() => root.deleteSync(recursive: true));
    return root;
  }

  Future<void> pumpReview(WidgetTester tester, String deck) async {
    final root = deckRoot(deck);
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: ReviewScreen(
          deckPath: '${root.path}/facts.md',
          rootDir: root.path,
          depth: ReviewDepth.recall,
          supportDir: tempSupport(),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> tapChip(WidgetTester tester, String label) async {
    final chip = find.widgetWithText(InkWell, label);
    expect(chip, findsOneWidget, reason: 'the $label chip is offered');
    await tester.ensureVisible(chip);
    await tester.tap(chip);
    await tester.pumpAndSettle();
  }

  final sheetTitle = find.byKey(const ValueKey('topic-sheet-title'));
  final questionTap = find.byKey(const ValueKey('question-tap'));
  final questionRegion = find.byKey(const ValueKey('question-region'));
  final pill = find.byKey(const ValueKey('topic-pill'));

  Future<void> dismissSheet(WidgetTester tester) async {
    expect(sheetTitle, findsOneWidget, reason: 'a sheet is open to dismiss');
    Navigator.of(tester.element(sheetTitle)).pop();
    await tester.pumpAndSettle();
  }

  Future<void> openContext(WidgetTester tester) => tapChip(tester, 'Context');

  // Card 1 introduces the topic this sitting and opens the sheet by
  // itself; card 2 is the steady state: the sheet waits for a tap.
  Future<void> advanceToSecondCard(WidgetTester tester) async {
    await dismissSheet(tester);
    await tapChip(tester, 'Reveal');
    await tapChip(tester, 'Seen');
  }

  String withTopic({String heading = _heading}) =>
      '---\ntitle: Roads\n---\n# $heading\n$_prose\n\n## q1?\na1\n\n## q2?\na2\n';

  testWidgets(
    'a card under a topic names it only by a Context chip: no pill, no heading, no prose, no question tap',
    (tester) async {
      await pumpReview(tester, withTopic());
      await advanceToSecondCard(tester);

      expect(pill, findsNothing, reason: 'the title pill is gone');
      expect(
        find.text(_heading),
        findsNothing,
        reason: 'the heading shows only inside the sheet',
      );
      expect(find.text('Context'), findsOneWidget, reason: 'the chip before the reveal');
      expect(
        find.text(_prose),
        findsNothing,
        reason: 'a card after the first shows the topic only on demand',
      );
      expect(questionTap, findsNothing, reason: 'the question is no tap target');
      await tester.tap(questionRegion);
      await tester.pumpAndSettle();
      expect(sheetTitle, findsNothing, reason: 'a tap on the question opens nothing');
    },
  );

  testWidgets('a topic-less card has no tap target and a tap on its question opens nothing', (
    tester,
  ) async {
    await pumpReview(tester, '---\ntitle: Plain\n---\n## q?\na\n');

    expect(questionTap, findsNothing, reason: 'nothing to open');
    expect(pill, findsNothing);
    expect(find.text('Context'), findsNothing);
    await tester.tap(questionRegion);
    await tester.pumpAndSettle();
    expect(sheetTitle, findsNothing, reason: 'no sheet opened');
  });

  testWidgets('the Context chip opens the topic sheet with heading and prose', (
    tester,
  ) async {
    await pumpReview(tester, withTopic());
    await advanceToSecondCard(tester);
    await openContext(tester);

    expect(sheetTitle, findsOneWidget, reason: 'the sheet opened');
    expect((tester.widget(sheetTitle) as Text).data, _heading);
    expect(find.text(_prose), findsOneWidget, reason: 'the prose is in the sheet');
  });

  testWidgets('the open sheet carries a close mark in its top-right corner, and a tap on it closes the sheet', (
    tester,
  ) async {
    await pumpReview(tester, withTopic());
    await advanceToSecondCard(tester);
    await openContext(tester);

    final closeMark = find.byKey(const ValueKey('topic-sheet-close'));
    expect(closeMark, findsOneWidget, reason: 'the sheet shows a close mark');
    final sheet = tester.getRect(find.byType(TopicSheet));
    final mark = tester.getRect(closeMark);
    expect(
      sheet.right - mark.right,
      lessThanOrEqualTo(24),
      reason: 'the mark sits at the sheet\'s right edge: sheet=$sheet mark=$mark',
    );
    expect(
      mark.top - sheet.top,
      lessThanOrEqualTo(24),
      reason: 'the mark sits at the sheet\'s top edge: sheet=$sheet mark=$mark',
    );
    expect(
      mark.left,
      greaterThan(tester.getRect(sheetTitle).left),
      reason: 'the mark is right of the heading, not above it',
    );

    await tester.tap(closeMark);
    await tester.pumpAndSettle();
    expect(sheetTitle, findsNothing, reason: 'the close mark dismissed the sheet');
    await openContext(tester);
    expect(sheetTitle, findsOneWidget, reason: 'the chip reopens it afterwards');
  });

  testWidgets('a topic too long for the screen keeps the sheet and its close mark below the status bar', (
    tester,
  ) async {
    addTearDown(tester.view.reset);
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3.0;
    tester.view.padding = const FakeViewPadding(top: 24 * 3);
    final longProse = List.filled(60, 'Applies on public roads and private ones.').join('\n\n');
    await pumpReview(
      tester,
      '---\ntitle: Roads\n---\n# $_heading\n$longProse\n\n## q1?\na1\n\n## q2?\na2\n',
    );
    await advanceToSecondCard(tester);
    await openContext(tester);

    final toolbarTop = tester.getRect(find.byType(AppBar)).bottom - kToolbarHeight;
    final sheet = tester.getRect(find.byType(TopicSheet));
    final mark = tester.getRect(find.byKey(const ValueKey('topic-sheet-close')));
    expect(
      sheet.top,
      greaterThanOrEqualTo(toolbarTop),
      reason: 'the sheet stops at the session header\'s top edge: sheet=$sheet header top=$toolbarTop',
    );
    expect(
      mark.top,
      greaterThanOrEqualTo(toolbarTop),
      reason: 'the close mark is below the status bar: mark=$mark header top=$toolbarTop',
    );
  });

  testWidgets(
    'the first introduction from a topic opens the sheet by itself once; the Context chip reopens it until the reveal, and the second card waits for the chip',
    (tester) async {
      await pumpReview(tester, withTopic());

      expect(
        sheetTitle,
        findsOneWidget,
        reason: "card 1 is the sitting's first introduction from the topic",
      );
      expect((tester.widget(sheetTitle) as Text).data, _heading);
      expect(find.text(_prose), findsOneWidget, reason: 'the prose is in the sheet');

      await dismissSheet(tester);
      expect(sheetTitle, findsNothing, reason: 'dismissed');
      expect(find.text(_prose), findsNothing, reason: 'the prose is never on the card face');
      await openContext(tester);
      expect(sheetTitle, findsOneWidget, reason: 'the chip is the reopen route');
      await dismissSheet(tester);

      await tapChip(tester, 'Reveal');
      expect(sheetTitle, findsNothing, reason: 'a state rebuild on the same card does not reopen the sheet');
      expect(find.text('Context'), findsNothing, reason: 'no context after the reveal');

      await tapChip(tester, 'Seen');
      expect(sheetTitle, findsNothing, reason: 'card 2 from the same topic does not open the sheet');
      await openContext(tester);
      expect(sheetTitle, findsOneWidget, reason: 'card 2 opens its topic from the chip');
    },
  );

  testWidgets(
    'distinct topics with the same heading and front each auto-open once',
    (tester) async {
      await pumpReview(
        tester,
        '---\ntitle: Repeated\n---\n'
        '# Shared\nFirst topic.\n\n## same?\na1\n\n'
        '# Shared\nSecond topic.\n\n## same?\na2\n',
      );

      expect(sheetTitle, findsOneWidget, reason: 'the first topic opened');
      await dismissSheet(tester);
      await tapChip(tester, 'Reveal');
      await tapChip(tester, 'Seen');

      final openedAutomatically = sheetTitle.evaluate().isNotEmpty;
      if (!openedAutomatically) await openContext(tester);
      expect(
        find.text('Second topic.'),
        findsOneWidget,
        reason:
            'the current card carries a different topic context despite its repeated heading and front',
      );
      expect(
        openedAutomatically,
        isTrue,
        reason:
            'the next card belongs to a different topic, so its first introduction must open that topic even when heading and front text repeat',
      );
    },
  );

  testWidgets(
    'topic prose joins hard-wrapped lines into one paragraph and breaks at a blank line',
    (tester) async {
      final card = ReviewCardModel(
        front: 'q?',
        frontRuns: const [],
        back: const ['a'],
        backRuns: const [],
        backUnits: const [],
        answerSteps: const [],
        reshaped: false,
        note: const [],
        images: const [],
        imagesBack: const [],
        topic: const [_heading, 'Line one', 'line two.', '', 'Line three.'],
      );
      await tester.pumpWidget(
        MaterialApp(
          theme: alixDark(),
          home: TopicSheet(card: card),
        ),
      );

      expect(
        find.text('Line one line two.'),
        findsOneWidget,
        reason: 'a hard wrap is a space inside one paragraph',
      );
      expect(
        find.text('Line three.'),
        findsOneWidget,
        reason: 'a blank line starts a new paragraph',
      );
    },
  );

  testWidgets('a fence-shaped topic heading never opens a prose code block', (
    tester,
  ) async {
    await pumpReview(tester, withTopic(heading: '```text'));

    final prose = tester.widget<Text>(find.text(_prose));
    expect(
      prose.style?.fontFamily,
      isNot('IBM Plex Mono'),
      reason:
          'heading/prose boundary: expected ordinary topic prose, actual font family ${prose.style?.fontFamily}',
    );
  });
}
