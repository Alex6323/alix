import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/theme.dart';

const _question = 'What should remain visible when an authored answer is long?';

ReviewSentenceModel _sentence(String text) => ReviewSentenceModel(
  text: text,
  runs: [InlineRunModel(text: text, bold: false, italic: false, code: false)],
);

List<String> _lines(int count, String Function(int) text) =>
    List.generate(count, (index) => text(index + 1));

Widget _card({
  required List<String> answer,
  required int noteParagraphs,
  required TextEditingController attempt,
}) {
  final note = ReviewNoteModel(
    badge: ReviewBadge.note,
    units: [
      for (final text in _lines(
        noteParagraphs,
        (n) => 'Note paragraph $n explains one more thing the learner asked.',
      ))
        _sentence(text),
    ],
  );
  final card = ReviewCardModel(
    front: _question,
    frontRuns: [
      InlineRunModel(text: _question, bold: false, italic: false, code: false),
    ],
    context: const [],
    contextRuns: const [],
    back: answer,
    backRuns: [for (final _ in answer) const []],
    backUnits: [for (final text in answer) _sentence(text)],
    answerSteps: [ReviewAnswerLineModel(backFrom: 0, backTo: answer.length)],
    reshaped: false,
    note: [note],
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

Rect _rect(WidgetTester tester, String key) =>
    tester.getRect(find.byKey(ValueKey(key)));

void main() {
  const shortAnswer = ['A short answer.'];
  final longAnswer = _lines(
    40,
    (n) => 'Answer line $n keeps the explanation long enough to overflow.',
  );
  for (final (name, answer, paragraphs) in [
    ('a short answer and a long note', shortAnswer, 16),
    ('a long answer and a long note', longAnswer, 16),
    ('a short answer and a short note', shortAnswer, 1),
  ]) {
    testWidgets(
      'with $name the note region sits at the body bottom, sized to its '
      'content, and the answer region keeps a third of the body',
      (tester) async {
        addTearDown(tester.view.reset);
        tester.view.physicalSize = const Size(1080, 2280);
        tester.view.devicePixelRatio = 3.0;
        final attempt = TextEditingController();
        addTearDown(attempt.dispose);

        await tester.pumpWidget(
          _card(answer: answer, noteParagraphs: paragraphs, attempt: attempt),
        );
        await tester.pumpAndSettle();

        final body = _rect(tester, 'card-body');
        final question = _rect(tester, 'question-region');
        final answerRegion = _rect(tester, 'answer-region');
        final noteRegion = _rect(tester, 'note-region');
        final noteContent = _rect(tester, 'note-content');
        final floor = body.height / 3;

        expect(
          (noteRegion.bottom - body.bottom).abs(),
          lessThanOrEqualTo(1),
          reason:
              'the note bottom ${noteRegion.bottom} sits at the body bottom '
              '${body.bottom}',
        );
        expect(
          (noteRegion.top - answerRegion.bottom).abs(),
          lessThanOrEqualTo(1),
          reason:
              'no band between the answer bottom ${answerRegion.bottom} and '
              'the note top ${noteRegion.top}',
        );
        expect(
          question.height,
          lessThanOrEqualTo(body.height * 0.38 + 1),
          reason: 'the question region ${question.height} stays under 38% of '
              'the body ${body.height}',
        );
        expect(
          answerRegion.height,
          greaterThanOrEqualTo(floor - 1),
          reason:
              'the answer region ${answerRegion.height} keeps a third of the '
              'body $floor',
        );
        if (paragraphs == 1) {
          expect(
            (noteRegion.height - noteContent.height).abs(),
            lessThanOrEqualTo(1),
            reason:
                'a short note region ${noteRegion.height} equals its content '
                '${noteContent.height}',
          );
        } else {
          expect(
            noteContent.height,
            greaterThan(noteRegion.height + 1),
            reason: 'a long note scrolls inside its region',
          );
          expect(
            (answerRegion.height - floor).abs(),
            lessThanOrEqualTo(1),
            reason:
                'a long note pushes the answer region ${answerRegion.height} '
                'down to the floor $floor',
          );
        }
      },
    );
  }
}
