import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/theme.dart';

const _inset = 10.0;

// A Container adds its border to its own padding, so a framed note's text
// clears the answer edge by that one painted pixel.
const _frameBorder = 1.0;

const _question =
    'Which of the two clients decides the check a card is reviewed under, '
    'and what does the deck itself contribute to that decision?';
const _answer =
    'The chosen depth decides it, never the deck: Recognize is always a '
    'choice, Recall flips, and Reconstruct types or rebuilds.';
const _badged =
    'A badged note carries no ground of its own, so the divider and its own '
    'chip are what set it apart from the answer above it.';
const _plain =
    'A badgeless note keeps the plain note ground, which is the only '
    'separation it has, so it stays a framed box.';

ReviewSentenceModel _sentence(String text) => ReviewSentenceModel(
  text: text,
  runs: [InlineRunModel(text: text, bold: false, italic: false, code: false)],
);

ReviewNoteModel _note(String text, {ReviewBadge? badge}) =>
    ReviewNoteModel(badge: badge, units: [_sentence(text)]);

Widget _card(TextEditingController attempt) {
  final card = ReviewCardModel(
    front: _question,
    frontRuns: [
      InlineRunModel(text: _question, bold: false, italic: false, code: false),
    ],
    context: const [],
    contextLeads: false,
    contextRuns: const [],
    contextUnits: const [],
    back: const [_answer],
    backRuns: const [[]],
    backUnits: [_sentence(_answer)],
    answerSteps: const [ReviewAnswerLineModel(backFrom: 0, backTo: 1)],
    reshaped: false,
    note: [_note(_badged, badge: ReviewBadge.warning), _note(_plain)],
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

Finder get _noteBoxes => find.byWidgetPredicate(
  (widget) => widget is Container && widget.constraints?.maxWidth == 600,
);

void main() {
  testWidgets(
    'every left-aligned block sits one inset inside the question, and a '
    'framed note keeps its box on the question measure',
    (tester) async {
      tester.view.physicalSize = const Size(1080, 2280);
      tester.view.devicePixelRatio = 3.0;
      addTearDown(tester.view.reset);
      final attempt = TextEditingController();
      addTearDown(attempt.dispose);

      await tester.pumpWidget(_card(attempt));
      await tester.pumpAndSettle();

      final question = tester.getRect(find.text(_question, findRichText: true));
      final answer = tester.getRect(find.text(_answer, findRichText: true));
      final badged = tester.getRect(find.text(_badged, findRichText: true));
      final plain = tester.getRect(find.text(_plain, findRichText: true));
      final framed = tester.getRect(_noteBoxes.last);

      expect(answer.left, question.left + _inset, reason: 'answer left edge');
      expect(answer.right, question.right - _inset, reason: 'answer right edge');
      expect(
        badged.left,
        question.left + _inset,
        reason: 'unboxed note left edge',
      );
      expect(
        badged.right,
        question.right - _inset,
        reason: 'unboxed note right edge',
      );

      expect(framed.left, question.left, reason: 'framed note box left edge');
      expect(framed.right, question.right, reason: 'framed note box right edge');
      expect(
        plain.left,
        answer.left + _frameBorder,
        reason: 'a framed note puts its text on the answer edge, not its box',
      );
    },
  );
}
