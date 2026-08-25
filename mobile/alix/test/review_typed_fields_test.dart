import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/theme.dart';

ReviewCardModel _card(List<String> back) => ReviewCardModel(
  front: 'Question',
  frontRuns: const [],
  context: const [],
  contextLeads: false,
  contextRuns: const [],
  contextUnits: const [],
  back: back,
  backRuns: [for (final _ in back) const []],
  backUnits: const [],
  reshaped: false,
  note: const [],
  images: const [],
  imagesBack: const [],
);

ReviewStateModel _state(ReviewCardModel card, ReviewMode mode) =>
    ReviewStateModel(
      card: card,
      mode: mode,
      depth: ReviewDepth.reconstruct,
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

Future<List<String>> _pumpAndSubmit(
  WidgetTester tester,
  ReviewCardModel card,
  ReviewMode mode,
) async {
  final attempt = TextEditingController();
  addTearDown(attempt.dispose);
  final typed = [
    for (final _ in card.back) TextEditingController(),
  ];
  for (final controller in typed) {
    addTearDown(controller.dispose);
  }
  List<String> sent = const [];

  await tester.pumpWidget(
    MaterialApp(
      theme: alixDark(),
      home: Scaffold(
        body: ReviewCardView(
          state: _state(card, mode),
          revealed: false,
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
          typedControllers: typed,
          serverLive: false,
          tutorCard: null,
          verdictGrade: ReviewGrade.pass,
          onChoose: (_) {},
          onToggleChoice: (_) {},
          onSubmitChoices: () {},
          onCheck: (lines) => sent = lines,
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

  for (var index = 0; index < typed.length; index++) {
    typed[index].text = card.back[index];
  }
  await tester.tap(find.text('Submit'));
  await tester.pump();
  return sent;
}

void main() {
  // A grouped cloze (two blanks sharing a name) is ONE card asking both spans,
  // so its `back` carries a line per span and Typing is chosen for the whole
  // card. One field would leave the second span unanswerable.
  testWidgets('typing renders a field for every answer line, not one', (
    tester,
  ) async {
    final card = _card(const ['alpha', 'beta']);

    final sent = await _pumpAndSubmit(tester, card, ReviewMode.typing);

    expect(find.byType(TextField), findsNWidgets(2));
    expect(sent, const ['alpha', 'beta']);
  });

  testWidgets('a single-line typing card still renders one field', (
    tester,
  ) async {
    final card = _card(const ['Paris']);

    final sent = await _pumpAndSubmit(tester, card, ReviewMode.typing);

    expect(find.byType(TextField), findsOneWidget);
    expect(sent, const ['Paris']);
  });
}
