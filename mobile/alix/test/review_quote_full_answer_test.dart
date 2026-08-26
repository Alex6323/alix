import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/theme.dart';

// A non-empty run list per line: `_runsOrText` renders an EMPTY list as an
// empty widget, so a fixture without runs makes both assertions vacuous.
List<InlineRunModel> _runs(String text) => [
  InlineRunModel(text: text, bold: false, italic: false, code: false),
];

ReviewCardModel _quoteCard({required bool reshaped}) => ReviewCardModel(
  front: 'Question',
  frontRuns: const [],
  context: const [],
  contextLeads: false,
  contextRuns: const [],
  contextUnits: const [],
  back: const ['the claim', '> supporting quotation', '> continued quotation'],
  backRuns: [
    _runs('the claim'),
    _runs('> supporting quotation'),
    _runs('> continued quotation'),
  ],
  backUnits: [
    ReviewSentenceModel(text: 'the claim', runs: _runs('the claim')),
    ReviewQuoteModel([
      ReviewSentenceModel(
        text: 'supporting quotation continued quotation',
        runs: _runs('supporting quotation continued quotation'),
      ),
    ]),
  ],
  answerSteps: [
    const ReviewAnswerLineModel(backFrom: 0, backTo: 1),
    ReviewAnswerQuoteModel(
      backFrom: 1,
      backTo: 3,
      units: [
        ReviewSentenceModel(
          text: 'supporting quotation continued quotation',
          runs: _runs('supporting quotation continued quotation'),
        ),
      ],
    ),
  ],
  reshaped: reshaped,
  note: const [],
  images: const [],
  imagesBack: const [],
);

ReviewStateModel _state(ReviewMode mode, {required bool reshaped}) =>
    ReviewStateModel(
      card: _quoteCard(reshaped: reshaped),
      mode: mode,
      depth: mode == ReviewMode.explain
          ? ReviewDepth.reconstruct
          : ReviewDepth.recall,
      introducing: false,
      keypoints: mode == ReviewMode.explain ? const ['the claim'] : null,
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

Widget _app(ReviewMode mode, {required bool reshaped}) {
  final attempt = TextEditingController();
  return MaterialApp(
    theme: alixDark(),
    home: Scaffold(
      body: ReviewCardView(
        state: _state(mode, reshaped: reshaped),
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
      ),
    ),
  );
}

void main() {
  testWidgets('explain renders supporting prose as a quote block', (
    tester,
  ) async {
    await tester.pumpWidget(_app(ReviewMode.explain, reshaped: false));

    expect(find.text('> supporting quotation'), findsNothing);
    expect(
      find.text('supporting quotation continued quotation'),
      findsOneWidget,
    );
  });

  testWidgets(
    'a reshaped flip still renders supporting prose as a quote block',
    (tester) async {
      await tester.pumpWidget(_app(ReviewMode.flip, reshaped: true));

      expect(find.text('> supporting quotation'), findsNothing);
      expect(
        find.text('supporting quotation continued quotation'),
        findsOneWidget,
      );
    },
  );
}
