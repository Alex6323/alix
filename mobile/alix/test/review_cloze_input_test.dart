import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/theme.dart';

const _holeRun = InlineRunModel(
  text: '⍰ then ⍰',
  bold: false,
  italic: false,
  code: false,
);

const _plainRun = InlineRunModel(
  text: 'plain question',
  bold: false,
  italic: false,
  code: false,
);

ReviewCardModel _card({
  List<InlineRunModel> frontRuns = const [_holeRun],
  List<ReviewContentUnitModel>? frontUnits,
  List<int> holeSteps = const [1, 0],
  bool cloze = true,
}) {
  final steps = cloze
      ? [
          ReviewFrontStepModel(
            front: 'alpha then ⍰',
            frontRuns: const [_holeRun],
          ),
          ReviewFrontStepModel(
            front: 'alpha then beta',
            frontRuns: const [_holeRun],
          ),
        ]
      : <ReviewFrontStepModel>[];
  return ReviewCardModel(
    front: cloze ? '⍰ then ⍰' : 'plain question',
    frontRuns: frontRuns,
    frontUnits: frontUnits,
    frontSteps: steps,
    holeSteps: holeSteps,
    back: const ['alpha', 'beta'],
    backRuns: const [[], []],
    backUnits: const [],
    answerSteps: const [
      ReviewAnswerLineModel(backFrom: 0, backTo: 1),
      ReviewAnswerLineModel(backFrom: 1, backTo: 2),
    ],
    reshaped: false,
    note: const [],
    images: const [],
    imagesBack: const [],
  );
}

ReviewStateModel _state(ReviewCardModel card, {ReviewDepth? depth}) {
  return ReviewStateModel(
    card: card,
    mode: depth == ReviewDepth.recognize
        ? ReviewMode.choice
        : ReviewMode.typing,
    depth: depth ?? ReviewDepth.reconstruct,
    introducing: false,
    choices: depth == ReviewDepth.recognize ? const ['alpha', 'beta'] : null,
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
}

class _Harness {
  _Harness(this.typed);

  final List<TextEditingController> typed;
  final List<List<String>> submissions = [];
}

Future<_Harness> _pump(
  WidgetTester tester,
  ReviewCardModel card, {
  ReviewCheckFeedbackModel? feedback,
  ReviewDepth? depth,
}) async {
  addTearDown(tester.view.reset);
  tester.view.physicalSize = const Size(1080, 2280);
  tester.view.devicePixelRatio = 3;
  final attempt = TextEditingController();
  addTearDown(attempt.dispose);
  final typed = [for (final _ in card.gradeableSteps) TextEditingController()];
  for (final controller in typed) {
    addTearDown(controller.dispose);
  }
  final harness = _Harness(typed);
  await tester.pumpWidget(
    MaterialApp(
      theme: alixDark(),
      home: Scaffold(
        body: ReviewCardView(
          state: _state(card, depth: depth),
          revealed: false,
          revealedLines: 0,
          choice: null,
          multiChoice: null,
          multiSelected: const {},
          checkFeedback: feedback,
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
          typedControllers: typed,
          serverLive: false,
          tutorCard: null,
          verdictGrade: ReviewGrade.pass,
          onChoose: (_) {},
          onToggleChoice: (_) {},
          onSubmitChoices: () {},
          onCheck: (lines) => harness.submissions.add(lines),
          onOpenAttempt: () {},
          onToggleKeypoint: (_) {},
          onReveal: () {},
          onRevealNextLine: () {},
          onIntroduce: () {},
          onGrade: (_) {},
          onOpenTutor: (_) {},
          onOpenTopic: (_) {},
        ),
      ),
    ),
  );
  await tester.pump();
  return harness;
}

Iterable<TextSpan> _textSpans(WidgetTester tester) sync* {
  Iterable<TextSpan> walk(InlineSpan span) sync* {
    if (span is TextSpan) {
      yield span;
      for (final child in span.children ?? const <InlineSpan>[]) {
        yield* walk(child);
      }
    }
  }

  for (final rich in tester.widgetList<RichText>(find.byType(RichText))) {
    yield* walk(rich.text);
  }
}

void main() {
  testWidgets('inline fields follow hole steps instead of reading order', (
    tester,
  ) async {
    final card = _card();
    final harness = await _pump(tester, card);

    final first = tester.widget<TextField>(
      find.byKey(const ValueKey('cloze-field-0')),
    );
    final second = tester.widget<TextField>(
      find.byKey(const ValueKey('cloze-field-1')),
    );
    expect(first.controller, same(harness.typed[1]));
    expect(second.controller, same(harness.typed[0]));
    first.controller!.text = 'beta';
    second.controller!.text = 'alpha';
    expect(first.controller!.text, 'beta');
    expect(second.controller!.text, 'alpha');
  });

  testWidgets(
    'the keyboard action steps through the blanks and the last one submits in step order',
    (tester) async {
      final card = _card();
      final harness = await _pump(tester, card);
      final first = tester.widget<TextField>(
        find.byKey(const ValueKey('cloze-field-0')),
      );
      final second = tester.widget<TextField>(
        find.byKey(const ValueKey('cloze-field-1')),
      );
      first.controller!.text = 'beta';
      second.controller!.text = 'alpha';
      bool focused(String key) => tester
          .widget<EditableText>(
            find.descendant(
              of: find.byKey(ValueKey(key)),
              matching: find.byType(EditableText),
            ),
          )
          .focusNode
          .hasFocus;

      await tester.tap(find.byKey(const ValueKey('cloze-field-0')));
      await tester.pump();
      expect(first.textInputAction, TextInputAction.next);
      await tester.testTextInput.receiveAction(TextInputAction.next);
      await tester.pump();
      expect(harness.submissions, isEmpty, reason: 'the first blank moves on');
      expect(
        focused('cloze-field-1'),
        isTrue,
        reason: 'the next blank has focus',
      );

      expect(second.textInputAction, TextInputAction.done);
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      expect(harness.submissions, const [
        ['alpha', 'beta'],
      ]);
    },
  );

  testWidgets(
    'inline feedback marks passes and strikes misses in the question',
    (tester) async {
      final card = _card();
      final feedback = ReviewCheckFeedbackModel(
        results: const [
          ReviewTypedResultModel(
            input: 'alpha',
            expected: 'alpha',
            passed: true,
          ),
          ReviewTypedResultModel(
            input: 'wrong',
            expected: 'beta',
            passed: false,
          ),
        ],
        passed: false,
      );
      await _pump(tester, card, feedback: feedback);

      expect(find.byType(TextField), findsNothing);
      final spans = _textSpans(tester).toList();
      final alpha = spans.singleWhere((span) => span.text == 'alpha');
      final wrong = spans.singleWhere((span) => span.text == 'wrong');
      final beta = spans.singleWhere((span) => span.text == 'beta');
      expect(alpha.style?.color, alixDark().alix.good);
      expect(wrong.style?.color, alixDark().alix.again);
      expect(wrong.style?.decoration, TextDecoration.lineThrough);
      expect(beta.style?.color, alixDark().alix.boltHi);
      expect(spans.indexOf(wrong), lessThan(spans.indexOf(alpha)));
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('answer-region')),
          matching: find.textContaining('alpha', findRichText: true),
        ),
        findsNothing,
      );
    },
  );

  testWidgets('a fenced blank keeps the separate answer fields', (
    tester,
  ) async {
    final card = _card(
      frontRuns: const [],
      frontUnits: [
        ReviewCodeModel(const ['let ⍰ = beta;']),
      ],
      holeSteps: const [],
    );
    await _pump(tester, card);

    expect(find.byKey(const ValueKey('cloze-field-0')), findsNothing);
    expect(find.byType(TextField), findsNWidgets(2));
  });

  testWidgets('only cloze cards omit the question divider', (tester) async {
    await _pump(tester, _card(holeSteps: const []));
    expect(find.byKey(const ValueKey('card-divider')), findsNothing);

    await _pump(
      tester,
      _card(frontRuns: const [_plainRun], holeSteps: const [], cloze: false),
    );
    expect(find.byKey(const ValueKey('card-divider')), findsOneWidget);
  });

  testWidgets('a cloze question is not capped at 38 percent of the body', (
    tester,
  ) async {
    final long = List.filled(
      90,
      'a long cloze question keeps its own reading space',
    ).join(' ');
    final card = _card(
      frontRuns: [
        InlineRunModel(
          text: '⍰ $long',
          bold: false,
          italic: false,
          code: false,
        ),
      ],
      holeSteps: const [0],
    );
    await _pump(tester, card);

    final body = tester.getRect(find.byKey(const ValueKey('card-body')));
    final question = tester.getRect(
      find.byKey(const ValueKey('question-region')),
    );
    expect(question.height, greaterThan(body.height * 0.38 + 1));
  });
}
