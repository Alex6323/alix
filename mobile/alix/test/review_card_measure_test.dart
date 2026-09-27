import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/theme.dart';

// The card's own reading column, mirrored from `_cardMeasure`.
const _measure = 560.0;

// A note holds its text off its bar, as the web's `.note` does; a Container
// adds its border to its own padding.
const _bar = 4.0;
const _barGap = 14.0;

const _question =
    'Which of the two clients decides the check a card is reviewed under, '
    'and what does the deck itself contribute to that decision?';

// The suite loads no fonts, so every glyph measures the same fixed width.
// This has to stay short enough to leave slack on ONE line at the narrowest
// measure under test, or it wraps, fills the column, and the assertion that
// an answer unit does not shrink-wrap can no longer fail.
const _short = 'Recall flips.';
const _answer =
    'The chosen depth decides it, never the deck: Recognize is always a '
    'choice, Recall flips, and Reconstruct types or rebuilds.';
const _warning =
    'A note carries no ground of its own, so its bar and its badge word are '
    'what set it apart from the answer above it.';
const _plain =
    'A plain note is a NOTE like any other, marked by the same bar in the '
    'colour GitHub gives that badge.';

ReviewSentenceModel _sentence(String text) => ReviewSentenceModel(
  text: text,
  runs: [InlineRunModel(text: text, bold: false, italic: false, code: false)],
);

ReviewNoteModel _note(String text, {ReviewBadge badge = ReviewBadge.note}) =>
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
    back: const [_short, _answer],
    backRuns: const [[], []],
    backUnits: [_sentence(_short), _sentence(_answer)],
    answerSteps: const [ReviewAnswerLineModel(backFrom: 0, backTo: 2)],
    reshaped: false,
    note: [_note(_warning, badge: ReviewBadge.warning), _note(_plain)],
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

Rect _textRect(WidgetTester tester, String text) =>
    tester.getRect(find.text(text, findRichText: true));

Future<void> _pump(
  WidgetTester tester,
  TextEditingController attempt,
  Size physical,
) async {
  tester.view.physicalSize = physical;
  tester.view.devicePixelRatio = 3.0;
  await tester.pumpWidget(_card(attempt));
  await tester.pumpAndSettle();
}

void main() {
  // The device's own geometry in both orientations: portrait never reaches
  // the measure, landscape is wider than it, so the cap is exercised once.
  for (final (name, physical) in const [
    ('portrait', Size(1080, 2280)),
    ('landscape', Size(2280, 1080)),
  ]) {
    testWidgets(
      'in $name the question, every answer unit and every note wrap on one '
      'measure, and a note holds its text off its bar',
      (tester) async {
        addTearDown(tester.view.reset);
        final attempt = TextEditingController();
        addTearDown(attempt.dispose);

        await _pump(tester, attempt, physical);

        final question = _textRect(tester, _question);
        final short = _textRect(tester, _short);
        final answer = _textRect(tester, _answer);
        final plain = _textRect(tester, _plain);
        // A note's own block is the innermost Container around its body.
        Rect block(String body) => tester.getRect(
          find
              .ancestor(
                of: find.text(body, findRichText: true),
                matching: find.byType(Container),
              )
              .first,
        );

        expect(question.width, lessThanOrEqualTo(_measure));
        for (final (label, rect) in [
          ('a wrapping answer unit', answer),
          ('a one-line answer unit', short),
          ('a WARNING note block', block(_warning)),
          ('a NOTE note block', block(_plain)),
        ]) {
          expect(rect.left, question.left, reason: '$label left edge');
          expect(rect.right, question.right, reason: '$label right edge');
        }
        expect(
          plain.left,
          block(_plain).left + _bar + _barGap,
          reason: 'a note holds its text off its bar',
        );
        expect(
          plain.right,
          lessThanOrEqualTo(block(_plain).right),
          reason: 'a note runs to the measure on the right',
        );
      },
    );
  }
}
