import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/theme.dart';

void main() {
  testWidgets('each note renders in its own box and keeps authored order', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    final card = ReviewCardModel(
      front: 'Question',
      frontRuns: const [],
      context: const [],
      contextLeads: false,
      contextRuns: const [],
      contextUnits: const [],
      back: const ['Answer'],
      backRuns: const [[]],
      backUnits: const [],
      answerSteps: const [ReviewAnswerLineModel(backFrom: 0, backTo: 1)],
      reshaped: false,
      note: [
        ReviewNoteModel(
          badge: ReviewBadge.warning,
          units: [
            ReviewSentenceModel(
              text: 'First.',
              runs: const [
                InlineRunModel(
                  text: 'First.',
                  bold: false,
                  italic: false,
                  code: false,
                ),
              ],
            ),
          ],
        ),
        ReviewNoteModel(
          units: [
            ReviewSentenceModel(
              text: 'Second.',
              runs: const [
                InlineRunModel(
                  text: 'Second.',
                  bold: false,
                  italic: false,
                  code: false,
                ),
              ],
            ),
          ],
        ),
      ],
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

    await tester.pumpWidget(
      MaterialApp(
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
      ),
    );

    expect(find.text('First.'), findsOneWidget);
    expect(find.text('Second.'), findsOneWidget);
    expect(
      find.byWidgetPredicate(
        (widget) =>
            widget is Container &&
            widget.constraints?.maxWidth == 600,
      ),
      findsNWidgets(2),
    );
  });
}
