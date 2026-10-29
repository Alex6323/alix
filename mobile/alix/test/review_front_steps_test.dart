import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/shared/inline_models.dart';

ReviewCardModel card({required int frontSteps, int answerSteps = 3}) {
  return ReviewCardModel(
    front: 'masked',
    frontRuns: const [],
    frontSteps: [
      for (var step = 1; step <= frontSteps; step++)
        ReviewFrontStepModel(
          front: 'step $step',
          frontRuns: [
            InlineRunModel(
              text: 'step $step',
              bold: false,
              italic: false,
              code: false,
              answer: true,
            ),
          ],
        ),
    ],
    back: [for (var line = 0; line < answerSteps; line++) 'a$line'],
    backRuns: [for (var line = 0; line < answerSteps; line++) const []],
    backUnits: const [],
    answerSteps: [
      for (var line = 0; line < answerSteps; line++)
        ReviewAnswerLineModel(backFrom: line, backTo: line + 1),
    ],
    reshaped: false,
    note: const [],
    images: const [],
    imagesBack: const [],
  );
}

void main() {
  test('a cloze card counts its reveal steps from its filled questions', () {
    expect(revealStepCount(card(frontSteps: 2)), 2, reason: 'cloze card');
    expect(revealStepCount(card(frontSteps: 0)), 3, reason: 'plain card');
  });

  test('every mode fills the question exactly when its answer is shown', () {
    final rows = <(String, ReviewMode, bool, bool, int, int)>[
      ('flip before reveal', ReviewMode.flip, false, false, 0, 0),
      ('flip after reveal', ReviewMode.flip, false, true, 0, 2),
      ('typing before check', ReviewMode.typing, false, false, 0, 0),
      ('typing at feedback', ReviewMode.typing, false, true, 0, 2),
      ('typeLine at feedback', ReviewMode.typeLine, false, true, 0, 2),
      ('choice before choosing', ReviewMode.choice, false, false, 0, 0),
      ('choice at feedback', ReviewMode.choice, false, true, 0, 2),
      ('explain after reveal', ReviewMode.explain, false, true, 0, 2),
      ('line before a press', ReviewMode.lineByLine, false, false, 0, 0),
      ('line after one press', ReviewMode.lineByLine, false, false, 1, 1),
      ('line after two presses', ReviewMode.lineByLine, false, true, 2, 2),
      ('line pressed past the end', ReviewMode.lineByLine, false, true, 5, 2),
      ('intro line before reveal', ReviewMode.lineByLine, true, false, 0, 0),
      ('intro line after reveal', ReviewMode.lineByLine, true, true, 0, 2),
      ('intro flip after reveal', ReviewMode.flip, true, true, 0, 2),
    ];
    for (final (label, mode, introducing, answered, lines, expected) in rows) {
      expect(
        filledFrontSteps(
          card(frontSteps: 2),
          mode: mode,
          introducing: introducing,
          answered: answered,
          revealedLines: lines,
        ),
        expected,
        reason: label,
      );
      expect(
        filledFrontSteps(
          card(frontSteps: 0),
          mode: mode,
          introducing: introducing,
          answered: answered,
          revealedLines: lines,
        ),
        0,
        reason: '$label on a card without blanks',
      );
    }
  });

  test('the question at a fill count is the masked card or that step', () {
    final cloze = card(frontSteps: 2);
    expect(questionAt(cloze, 0).front, 'masked');
    expect(questionAt(cloze, 1).front, 'step 1');
    expect(questionAt(cloze, 2).front, 'step 2');
  });
}
