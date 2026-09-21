import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/shared/inline_runs.dart';
import 'package:alix_mobile/theme.dart';

ReviewNoteModel _note(String text, {ReviewBadge? badge}) => ReviewNoteModel(
  badge: badge,
  units: [
    ReviewSentenceModel(
      text: text,
      runs: [
        InlineRunModel(text: text, bold: false, italic: false, code: false),
      ],
    ),
  ],
);

Widget _card(
  List<ReviewNoteModel> notes,
  TextEditingController attempt, {
  List<ReviewContentUnitModel>? frontUnits,
}) {
  final card = ReviewCardModel(
    front: 'Question',
    frontRuns: const [],
    frontUnits: frontUnits,
    context: const [],
    contextLeads: false,
    contextRuns: const [],
    contextUnits: const [],
    back: const ['Answer'],
    backRuns: const [[]],
    backUnits: const [],
    answerSteps: const [ReviewAnswerLineModel(backFrom: 0, backTo: 1)],
    reshaped: false,
    note: notes,
    images: const [],
    imagesBack: const [],
  );
  final state = ReviewStateModel(
    card: card,
    mode: ReviewMode.flip,
    depth: ReviewDepth.recall,
    introducing: false,
    finished: false,
    remaining: 1,
    reviews: 0,
    passed: 0,
    failed: 0,
    introduced: 0,
    partial: 0,
    canRestart: false,
    dueLeft: 1,
    newLeft: 0,
  );
  return MaterialApp(
    theme: alixDark(),
    home: Scaffold(
      body: ReviewCardView(
        state: state,
        revealed: true,
        revealedLines: 0,
        choice: null,
        multiChoice: null,
        multiSelected: const {},
        checkFeedback: null,
        typelineChecked: const [],
        tickedKeypoints: const {},
        sketch: Sketch(),
        onSketchBegin: (_, _) {},
        onSketchExtend: (_) {},
        onSketchEnd: () {},
        onSketchTool: (_) {},
        onSketchUndo: () {},
        onSketchClear: () {},
        attemptOpen: false,
        attemptController: attempt,
        typedControllers: const [],
        serverLive: false,
        tutorCard: null,
        verdictGrade: ReviewGrade.pass,
        onChoose: (_) {},
        onToggleChoice: (_) {},
        onSubmitChoices: () {},
        onCheck: (_) {},
        onOpenAttempt: () {},
        onToggleKeypoint: (_) {},
        onReveal: () {},
        onRevealNextLine: () {},
        onIntroduce: () {},
        onGrade: (_) {},
        onOpenTutor: (_) {},
        onOpenSection: (_) {},
      ),
    ),
  );
}

// A note's own box is the innermost Container around its body: the block
// carries no width of its own any more, so there is nothing else to key on.
Finder _noteBox(String body) =>
    find.ancestor(of: find.text(body), matching: find.byType(Container)).first;

Finder _badgeChip(ReviewBadge badge) => find.byWidgetPredicate(
  (widget) =>
      widget is Container &&
      widget.child is Text &&
      (widget.child! as Text).data == badge.name.toUpperCase(),
);

Color _boxColour(WidgetTester tester, String body) {
  final box = tester.widget<Container>(_noteBox(body));
  return (box.decoration! as BoxDecoration).color!;
}

/// A badged note carries no ground of its own, so its accent is read from the
/// chip's border, which is where the hue moved.
Color _chipAccent(WidgetTester tester, ReviewBadge badge) {
  final chip = tester.widget<Container>(_badgeChip(badge));
  return ((chip.decoration! as BoxDecoration).border! as Border).top.color;
}

void main() {
  testWidgets('each note renders in its own box and keeps authored order', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);

    await tester.pumpWidget(
      _card([
        _note('First.', badge: ReviewBadge.warning),
        _note('Second.'),
      ], attempt),
    );

    expect(find.text('First.'), findsOneWidget);
    expect(find.text('Second.'), findsOneWidget);
    expect(_noteBox('First.'), findsOneWidget);
    expect(_noteBox('Second.'), findsOneWidget);
    expect(
      tester.getRect(_noteBox('First.')),
      isNot(tester.getRect(_noteBox('Second.'))),
      reason: 'two notes stack in their own boxes instead of merging',
    );
  });

  testWidgets('a quotation inside a note uses the note prose alignment', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    const quoted = 'A quoted passage.';
    final note = ReviewNoteModel(
      badge: null,
      units: [
        ReviewQuoteModel([
          ReviewSentenceModel(
            text: quoted,
            runs: [
              InlineRunModel(
                text: quoted,
                bold: false,
                italic: false,
                code: false,
              ),
            ],
          ),
        ]),
      ],
    );

    await tester.pumpWidget(_card([note], attempt));

    final runs = tester.widget<InlineRuns>(
      find
          .ancestor(of: find.text(quoted), matching: find.byType(InlineRuns))
          .first,
    );
    expect(runs.textAlign, TextAlign.justify);
  });

  testWidgets('a checklist inside a note uses the note prose alignment', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    const itemText = 'A checklist item that belongs to the authored note.';
    final note = ReviewNoteModel(
      badge: null,
      units: [
        ReviewChecklistModel([
          ReviewChecklistItemModel(
            checked: true,
            text: itemText,
            runs: const [
              InlineRunModel(
                text: itemText,
                bold: false,
                italic: false,
                code: false,
              ),
            ],
          ),
        ]),
      ],
    );

    await tester.pumpWidget(_card([note], attempt));

    final runs = tester.widget<InlineRuns>(
      find
          .ancestor(of: find.text(itemText), matching: find.byType(InlineRuns))
          .first,
    );
    expect(runs.textAlign, TextAlign.justify);
  });

  testWidgets('a checklist on the centred question keeps its rows at the start', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    const itemText = 'A checklist item that belongs to the question.';

    await tester.pumpWidget(
      _card(
        const [],
        attempt,
        frontUnits: [
          ReviewChecklistModel([
            ReviewChecklistItemModel(
              checked: false,
              text: itemText,
              runs: const [
                InlineRunModel(
                  text: itemText,
                  bold: false,
                  italic: false,
                  code: false,
                ),
              ],
            ),
          ]),
        ],
      ),
    );

    final runs = tester.widget<InlineRuns>(
      find
          .ancestor(of: find.text(itemText), matching: find.byType(InlineRuns))
          .first,
    );
    expect(runs.textAlign, TextAlign.start);
  });

  testWidgets('a quotation on the question keeps its prose justified', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    const quoted = 'A quoted passage that belongs to the question.';

    await tester.pumpWidget(
      _card(
        const [],
        attempt,
        frontUnits: [
          ReviewQuoteModel([
            ReviewSentenceModel(
              text: quoted,
              runs: [
                InlineRunModel(
                  text: quoted,
                  bold: false,
                  italic: false,
                  code: false,
                ),
              ],
            ),
          ]),
        ],
      ),
    );

    final runs = tester.widget<InlineRuns>(
      find
          .ancestor(of: find.text(quoted), matching: find.byType(InlineRuns))
          .first,
    );
    expect(runs.textAlign, TextAlign.justify);
  });

  testWidgets('every badge names itself and marks its own accent', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    final tokens = alixDark().alix;
    final accents = {
      ReviewBadge.note: tokens.bolt,
      ReviewBadge.tip: tokens.good,
      ReviewBadge.important: tokens.bolt,
      ReviewBadge.warning: tokens.warn,
      ReviewBadge.caution: tokens.again,
    };

    for (final badge in ReviewBadge.values) {
      await tester.pumpWidget(_card([_note('Body.', badge: badge)], attempt));
      final name = badge.name.toUpperCase();
      expect(find.text(name), findsOneWidget, reason: '$badge names its chip');
      expect(
        _chipAccent(tester, badge),
        accents[badge]!.withValues(
          alpha: badge == ReviewBadge.important ? 1.0 : 0.55,
        ),
        reason: '$badge marks itself with its own accent',
      );
      // The accent paints borders, never small text: across the 21 palettes
      // accent-on-its-own-wash measures as low as 2.0:1.
      expect(
        tester.widget<Text>(find.text(name)).style!.color,
        tokens.dim,
        reason: "$badge's chip is inked, not accent-coloured",
      );
    }
  });

  testWidgets('the heavier important border does not change chip height', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);

    await tester.pumpWidget(
      _card([
        for (final badge in ReviewBadge.values)
          _note('Body.', badge: badge),
      ], attempt),
    );

    final heights = {
      for (final badge in ReviewBadge.values)
        badge.name: tester.getSize(_badgeChip(badge)).height,
    };
    expect(
      heights.values.toSet(),
      hasLength(1),
      reason:
          'a heavier border is a semantic cue, not a reason for IMPORTANT to '
          'shift its note body or break the shared chip rhythm: $heights',
    );
  });

  testWidgets('a badgeless note keeps the plain note ground and no chip', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    final tokens = alixDark().alix;

    await tester.pumpWidget(_card([_note('A table column.')], attempt));

    for (final badge in ReviewBadge.values) {
      expect(
        find.text(badge.name.toUpperCase()),
        findsNothing,
        reason: 'no badge, no chip',
      );
    }
    expect(
      _boxColour(tester, 'A table column.'),
      tokens.noteBorder.withValues(alpha: 0.12),
    );
  });
}
