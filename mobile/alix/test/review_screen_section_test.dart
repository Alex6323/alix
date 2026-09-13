// The section affordance on the phone: a card with a section carries a
// one-line title above the prompt and a quiet "Context" chip in the legend
// row that opens the section in a bottom sheet; a section-less card carries
// neither. No pairing and no server are involved: the section rides in the
// deck. ReviewScreen calls the real bridge, so RustLib.init() is required.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

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

  // Card 1 introduces the section this sitting and may arrive with it
  // inline; card 2 is the steady state: the title line and the chip.
  Future<void> advanceToSecondCard(WidgetTester tester) async {
    await tapChip(tester, 'Reveal');
    await tapChip(tester, 'Seen');
  }

  String sectioned({String heading = _heading}) =>
      '---\ntitle: Roads\n---\n# $heading\n$_prose\n\n## q1?\na1\n\n## q2?\na2\n';

  testWidgets(
    'a sectioned card shows the title line and the Context chip before any attempt',
    (tester) async {
      await pumpReview(tester, sectioned());
      await advanceToSecondCard(tester);

      final title = find.byKey(const ValueKey('section-title'));
      expect(title, findsOneWidget, reason: 'the title line is on the card');
      expect(
        (tester.widget(title) as Text).data,
        _heading,
        reason: 'the title line carries the section heading text',
      );
      expect(
        find.text('Context'),
        findsOneWidget,
        reason: 'the chip is offered',
      );
      expect(
        find.text(_prose),
        findsNothing,
        reason: 'a card after the first shows the section only on demand',
      );
    },
  );

  testWidgets('a section-less card shows neither the title line nor the chip', (
    tester,
  ) async {
    await pumpReview(tester, '---\ntitle: Plain\n---\n## q?\na\n');

    expect(find.byKey(const ValueKey('section-title')), findsNothing);
    expect(find.text('Context'), findsNothing);
  });

  testWidgets(
    'the Context chip opens the section sheet with heading and prose',
    (tester) async {
      await pumpReview(tester, sectioned());
      await advanceToSecondCard(tester);
      await tapChip(tester, 'Context');

      final sheetTitle = find.byKey(const ValueKey('section-sheet-title'));
      expect(sheetTitle, findsOneWidget, reason: 'the sheet opened');
      expect((tester.widget(sheetTitle) as Text).data, _heading);
      expect(
        find.text(_prose),
        findsOneWidget,
        reason: 'the prose is in the sheet',
      );
    },
  );

  testWidgets('the title line is one line that truncates, never wraps', (
    tester,
  ) async {
    await pumpReview(tester, sectioned(heading: _longHeading));
    await advanceToSecondCard(tester);

    final title = tester.widget<Text>(
      find.byKey(const ValueKey('section-title')),
    );
    expect(title.maxLines, 1, reason: 'one line');
    expect(title.overflow, TextOverflow.ellipsis, reason: 'ellipsis, no wrap');
  });
}
