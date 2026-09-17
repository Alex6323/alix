import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/theme.dart';

ReviewStateModel _state({
  required ReviewMode mode,
  bool introducing = false,
  List<String>? choices,
  bool? multiple,
}) => ReviewStateModel(
  mode: mode,
  depth: ReviewDepth.recall,
  introducing: introducing,
  choices: choices,
  choicesMultiple: multiple,
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

void main() {
  test('every check names itself, and introducing outranks the mode', () {
    final cases = <String, ReviewStateModel>{
      'flip': _state(mode: ReviewMode.flip),
      'line': _state(mode: ReviewMode.lineByLine),
      'typing': _state(mode: ReviewMode.typing),
      'typing · line': _state(mode: ReviewMode.typeLine),
      'explain': _state(mode: ReviewMode.explain),
      'choice': _state(
        mode: ReviewMode.choice,
        choices: const ['a', 'b'],
        multiple: false,
      ),
      'select all': _state(
        mode: ReviewMode.choice,
        choices: const ['a', 'b'],
        multiple: true,
      ),
    };

    cases.forEach((expected, state) {
      expect(reviewModeLabel(state), expected, reason: '$expected names itself');
    });

    // A card being introduced is ungraded whatever check would otherwise
    // apply, so that fact wins the one slot the bar has.
    for (final state in cases.values) {
      expect(
        reviewModeLabel(
          _state(
            mode: state.mode,
            introducing: true,
            choices: state.choices,
            multiple: state.choicesMultiple,
          ),
        ),
        'new',
      );
    }
  });

  testWidgets('the tag renders the label upper-cased and never wraps', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: const Scaffold(
          body: Center(child: ReviewModeTag(label: 'typing · line')),
        ),
      ),
    );

    expect(find.text('TYPING · LINE'), findsOneWidget);
    final text = tester.widget<Text>(find.text('TYPING · LINE'));
    expect(text.maxLines, 1, reason: 'the bar holds a fixed height');
    expect(
      text.overflow,
      TextOverflow.ellipsis,
      reason: 'a label too wide for the bar truncates, never reflows it',
    );
  });
}
