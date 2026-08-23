import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/theme.dart';

ReviewCardModel _card() {
  return ReviewCardModel(
    front: 'Which are even?',
    frontRuns: const [],
    context: const [],
    contextLeads: false,
    contextRuns: const [],
    contextUnits: const [],
    back: const ['two', 'four'],
    backRuns: const [[], []],
    backUnits: const [],
    reshaped: false,
    note: const [],
    images: const [],
    imagesBack: const [],
  );
}

ReviewStateModel _state() {
  return ReviewStateModel(
    card: _card(),
    mode: ReviewMode.choice,
    depth: ReviewDepth.recall,
    introducing: false,
    choices: const ['two', 'three', 'four'],
    choicesMultiple: true,
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

Widget _pump({
  required ReviewMultiChoiceFeedbackModel? multiChoice,
  required Set<int> multiSelected,
  required TextEditingController attempt,
  ValueChanged<int>? onToggleChoice,
  VoidCallback? onSubmitChoices,
  ValueChanged<ReviewGrade>? onGrade,
}) {
  return MaterialApp(
    theme: alixDark(),
    home: Scaffold(
      body: ReviewCardView(
        state: _state(),
        revealed: false,
        revealedLines: 0,
        choice: null,
        multiChoice: multiChoice,
        multiSelected: multiSelected,
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
        onToggleChoice: onToggleChoice ?? (_) {},
        onSubmitChoices: onSubmitChoices ?? () {},
        onCheck: (_) {},
        onOpenAttempt: () {},
        onToggleKeypoint: (_) {},
        onReveal: () {},
        onRevealNextLine: () {},
        onIntroduce: () {},
        onGrade: onGrade ?? (_) {},
        onOpenTutor: (_) {},
      ),
    ),
  );
}

void main() {
  testWidgets('a select-all card toggles picks and submits the set', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    final toggles = <int>[];
    var submits = 0;
    await tester.pumpWidget(
      _pump(
        multiChoice: null,
        multiSelected: const {0},
        attempt: attempt,
        onToggleChoice: toggles.add,
        onSubmitChoices: () => submits++,
      ),
    );

    expect(find.text('SELECT ALL'), findsOneWidget);
    expect(find.text('Submit'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('option-2')));
    await tester.tap(find.byKey(const ValueKey('option-0')));
    expect(toggles, [2, 0], reason: 'taps toggle instead of grading');
    expect(submits, 0, reason: 'no submission before the Submit action');

    await tester.tap(find.text('Submit'));
    expect(submits, 1);
  });

  testWidgets('select-all feedback locks the options and shows the verdict', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    final toggles = <int>[];
    await tester.pumpWidget(
      _pump(
        multiChoice: ReviewMultiChoiceFeedbackModel(
          chosen: const {1},
          correct: const {0, 2},
          passed: false,
        ),
        multiSelected: const {1},
        attempt: attempt,
        onToggleChoice: toggles.add,
      ),
    );

    expect(find.text('Submit'), findsNothing);
    expect(find.text('Continue'), findsOneWidget);

    await tester.tap(
      find.byKey(const ValueKey('option-0')),
      warnIfMissed: false,
    );
    expect(toggles, isEmpty, reason: 'feedback locks the options');
  });

  testWidgets('a passed select-all set offers Next and I guessed', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    final grades = <ReviewGrade>[];
    await tester.pumpWidget(
      _pump(
        multiChoice: ReviewMultiChoiceFeedbackModel(
          chosen: const {0, 2},
          correct: const {0, 2},
          passed: true,
        ),
        multiSelected: const {0, 2},
        attempt: attempt,
        onGrade: grades.add,
      ),
    );

    expect(find.text('Next'), findsOneWidget);
    expect(find.text('I guessed'), findsOneWidget);
    await tester.tap(find.text('Next'));
    expect(grades, [ReviewGrade.pass]);
  });
}
