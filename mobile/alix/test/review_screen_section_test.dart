// The section affordance on the phone: a card with a section carries a
// tappable one-line title pill above the prompt that opens the section in a
// bottom sheet; the sitting's first introduction from a section opens that
// sheet by itself, once. A section-less card carries neither. No pairing and
// no server are involved: the section rides in the deck. ReviewScreen calls
// the real bridge, so RustLib.init() is required.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart' show SectionSheet;
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

import 'support/deck_fixture.dart';

const _heading = 'Vehicle safety';
const _prose = 'Applies on public roads.';
const _longHeading =
    'A heading long enough that one phone line cannot hold it without an ellipsis at the end';

void main() {
  setUpAll(() async => RustLib.init());

  Directory tempSupport() {
    final dir = Directory.systemTemp.createTempSync('alix-section-support-');
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  Directory deckRoot(String deck) {
    final root = Directory.systemTemp.createTempSync('alix-section-decks-');
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

  final sheetTitle = find.byKey(const ValueKey('section-sheet-title'));
  final pill = find.byKey(const ValueKey('section-pill'));
  final title = find.byKey(const ValueKey('section-title'));

  Future<void> dismissSheet(WidgetTester tester) async {
    expect(sheetTitle, findsOneWidget, reason: 'a sheet is open to dismiss');
    Navigator.of(tester.element(sheetTitle)).pop();
    await tester.pumpAndSettle();
  }

  // Card 1 introduces the section this sitting and opens the sheet by
  // itself; card 2 is the steady state: the pill alone.
  Future<void> advanceToSecondCard(WidgetTester tester) async {
    await dismissSheet(tester);
    await tapChip(tester, 'Reveal');
    await tapChip(tester, 'Seen');
  }

  String sectioned({String heading = _heading}) =>
      '---\ntitle: Roads\n---\n# $heading\n$_prose\n\n## q1?\na1\n\n## q2?\na2\n';

  testWidgets(
    'a sectioned card shows the title pill and no Context chip, and keeps the prose off the card',
    (tester) async {
      await pumpReview(tester, sectioned());
      await advanceToSecondCard(tester);

      expect(pill, findsOneWidget, reason: 'the title pill is on the card');
      expect(
        (tester.widget(title) as Text).data,
        _heading,
        reason: 'the pill carries the section heading text',
      );
      expect(find.text('Context'), findsNothing, reason: 'no chip');
      expect(
        find.text(_prose),
        findsNothing,
        reason: 'a card after the first shows the section only on demand',
      );
    },
  );

  testWidgets('a section-less card shows neither the title pill nor a sheet', (
    tester,
  ) async {
    await pumpReview(tester, '---\ntitle: Plain\n---\n## q?\na\n');

    expect(pill, findsNothing);
    expect(sheetTitle, findsNothing);
    expect(find.text('Context'), findsNothing);
  });

  testWidgets('tapping the title pill opens the section sheet with heading and prose', (
    tester,
  ) async {
    await pumpReview(tester, sectioned());
    await advanceToSecondCard(tester);
    await tester.tap(pill);
    await tester.pumpAndSettle();

    expect(sheetTitle, findsOneWidget, reason: 'the sheet opened');
    expect((tester.widget(sheetTitle) as Text).data, _heading);
    expect(find.text(_prose), findsOneWidget, reason: 'the prose is in the sheet');
  });

  testWidgets('the title pill is one line that truncates, never wraps', (
    tester,
  ) async {
    await pumpReview(tester, sectioned(heading: _longHeading));
    await advanceToSecondCard(tester);

    final text = tester.widget<Text>(title);
    expect(text.maxLines, 1, reason: 'one line');
    expect(text.overflow, TextOverflow.ellipsis, reason: 'ellipsis, no wrap');
  });

  testWidgets(
    'the first introduction from a section opens the sheet by itself once; reveal does not reopen it and the second card does not open it',
    (tester) async {
      await pumpReview(tester, sectioned());

      expect(
        sheetTitle,
        findsOneWidget,
        reason: "card 1 is the sitting's first introduction from the section",
      );
      expect((tester.widget(sheetTitle) as Text).data, _heading);
      expect(find.text(_prose), findsOneWidget, reason: 'the prose is in the sheet');

      await dismissSheet(tester);
      expect(sheetTitle, findsNothing, reason: 'dismissed');
      expect(pill, findsOneWidget, reason: 'the pill stays as the reopen route');
      expect(find.text(_prose), findsNothing, reason: 'the prose is never on the card face');

      await tapChip(tester, 'Reveal');
      expect(sheetTitle, findsNothing, reason: 'a state rebuild on the same card does not reopen the sheet');

      await tapChip(tester, 'Seen');
      expect(sheetTitle, findsNothing, reason: 'card 2 from the same section does not open the sheet');
      expect(pill, findsOneWidget, reason: 'card 2 keeps the pill');
    },
  );

  testWidgets(
    'distinct sections with the same heading and front each auto-open once',
    (tester) async {
      await pumpReview(
        tester,
        '---\ntitle: Repeated\n---\n'
        '# Shared\nFirst section.\n\n## same?\na1\n\n'
        '# Shared\nSecond section.\n\n## same?\na2\n',
      );

      expect(sheetTitle, findsOneWidget, reason: 'the first section opened');
      await dismissSheet(tester);
      await tapChip(tester, 'Reveal');
      await tapChip(tester, 'Seen');

      final openedAutomatically = sheetTitle.evaluate().isNotEmpty;
      if (!openedAutomatically) {
        await tester.tap(pill);
        await tester.pumpAndSettle();
      }
      expect(
        find.text('Second section.'),
        findsOneWidget,
        reason:
            'the current card carries a different section context despite its repeated heading and front',
      );
      expect(
        openedAutomatically,
        isTrue,
        reason:
            'the next card belongs to a different section, so its first introduction must open that section even when heading and front text repeat',
      );
    },
  );

  testWidgets(
    'section prose joins hard-wrapped lines into one paragraph and breaks at a blank line',
    (tester) async {
      final card = ReviewCardModel(
        front: 'q?',
        frontRuns: const [],
        context: const [],
        contextLeads: false,
        contextRuns: const [],
        contextUnits: const [],
        back: const ['a'],
        backRuns: const [],
        backUnits: const [],
        answerSteps: const [],
        reshaped: false,
        note: const [],
        images: const [],
        imagesBack: const [],
        section: const [_heading, 'Line one', 'line two.', '', 'Line three.'],
      );
      await tester.pumpWidget(
        MaterialApp(
          theme: alixDark(),
          home: SectionSheet(card: card),
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

  testWidgets('a fence-shaped section heading never opens a prose code block', (
    tester,
  ) async {
    await pumpReview(tester, sectioned(heading: '```text'));

    final prose = tester.widget<Text>(find.text(_prose));
    expect(
      prose.style?.fontFamily,
      isNot('IBM Plex Mono'),
      reason:
          'heading/prose boundary: expected ordinary section prose, actual font family ${prose.style?.fontFamily}',
    );
  });
}
