import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/theme.dart';

const _front = 'What does `let` without `mut` give you in Rust?';
const _line1 = 'An immutable binding: the name cannot be reassigned.';
const _line2 = 'Shadow it with let instead.';
const _note = 'Immutability is the default so a reader knows a name keeps its value.';

InlineRunModel _run(String text, {bool code = false}) =>
    InlineRunModel(text: text, bold: false, italic: false, code: code);

ReviewCardModel _card() => ReviewCardModel(
  front: _front,
  frontRuns: [_run(_front)],
  back: const [_line1, _line2],
  backRuns: [
    [_run(_line1)],
    [_run('Shadow it with '), _run('let', code: true), _run(' instead.')],
  ],
  backUnits: [
    ReviewSentenceModel(text: _line1, runs: [_run(_line1)]),
    ReviewSentenceModel(
      text: _line2,
      runs: [_run('Shadow it with '), _run('let', code: true), _run(' instead.')],
    ),
  ],
  answerSteps: const [
    ReviewAnswerLineModel(backFrom: 0, backTo: 1),
    ReviewAnswerLineModel(backFrom: 1, backTo: 2),
  ],
  reshaped: false,
  note: [
    ReviewNoteModel(
      badge: ReviewBadge.note,
      units: [ReviewSentenceModel(text: _note, runs: [_run(_note)])],
    ),
  ],
  images: const [],
  imagesBack: const [],
);

ReviewStateModel _state({List<String>? choices}) => ReviewStateModel(
  card: _card(),
  mode: choices == null ? ReviewMode.flip : ReviewMode.choice,
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
  choices: choices,
  choiceRuns: choices?.map((text) => [_run(text)]).toList(),
);

class _Harness extends StatelessWidget {
  const _Harness({required this.state, required this.revealed, required this.attempt});

  final ReviewStateModel state;
  final bool revealed;
  final TextEditingController attempt;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      theme: alixDark(),
      home: Scaffold(
        body: ReviewCardView(
          state: state,
          revealed: revealed,
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
}

final _answerTap = find.byKey(const ValueKey('answer-tap'));
final _questionRegion = find.byKey(const ValueKey('question-region'));
final _answerRegion = find.byKey(const ValueKey('answer-region'));

Future<void> _pump(WidgetTester tester, {bool revealed = true, List<String>? choices}) async {
  addTearDown(tester.view.reset);
  tester.view.physicalSize = const Size(1080, 2280);
  tester.view.devicePixelRatio = 3.0;
  final attempt = TextEditingController();
  addTearDown(attempt.dispose);
  await tester.pumpWidget(
    _Harness(state: _state(choices: choices), revealed: revealed, attempt: attempt),
  );
  await tester.pumpAndSettle();
}

final _hideSheet = find.byKey(const ValueKey('hide-sheet'));
final _hideClose = find.byKey(const ValueKey('hide-sheet-close'));

void main() {
  testWidgets(
    'law: a tap on the answer covers it with a sheet from the answer\'s top edge, the question stays visible, and the close mark shows the answer again',
    (tester) async {
      await _pump(tester);
      expect(_answerTap, findsOneWidget, reason: 'a revealed answer is the tap target');
      final answerTop = tester.getRect(_answerRegion).top;
      final question = tester.getRect(_questionRegion);

      await tester.tap(_answerTap);
      await tester.pumpAndSettle();

      expect(_hideSheet, findsOneWidget, reason: 'the tap opened the hide sheet');
      final sheet = tester.getRect(_hideSheet);
      expect(
        (sheet.top - answerTop).abs(),
        lessThanOrEqualTo(1),
        reason: 'the sheet starts at the answer region\'s top edge: sheet=$sheet answer top=$answerTop',
      );
      expect(sheet.bottom, 760, reason: 'the sheet reaches the bottom of the screen');
      expect(question.bottom, lessThanOrEqualTo(sheet.top), reason: 'the question stays visible');
      expect(
        find.descendant(of: _hideSheet, matching: find.byIcon(Icons.visibility_off_outlined)),
        findsOneWidget,
        reason: 'the sheet shows the eye-off glyph',
      );
      expect(
        find.descendant(of: _hideSheet, matching: find.text('Hide')),
        findsOneWidget,
        reason: 'the header names the view',
      );
      expect(
        find.descendant(of: _hideSheet, matching: find.text(_line1)),
        findsNothing,
        reason: 'the sheet does not show the answer',
      );
      final close = tester.getRect(_hideClose);
      expect(sheet.right - close.right, lessThanOrEqualTo(24), reason: 'the close mark sits right');

      await tester.tap(_hideClose);
      await tester.pumpAndSettle();
      expect(_hideSheet, findsNothing, reason: 'the close mark dismissed the sheet');
      expect(find.text(_line1), findsOneWidget, reason: 'the answer shows again');
    },
  );

  testWidgets('an unrevealed card and a choice card have no tap target', (tester) async {
    await _pump(tester, revealed: false);
    expect(_answerTap, findsNothing, reason: 'nothing to hide before the reveal');
    await _pump(tester, revealed: false, choices: ['a', 'b', 'c']);
    expect(_answerTap, findsNothing, reason: 'options are picked, not hidden');
  });
}
