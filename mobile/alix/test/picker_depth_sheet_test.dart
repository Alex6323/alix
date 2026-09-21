import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/picker_widgets.dart';
import 'package:alix_mobile/theme.dart';

Widget _sheet(void Function(PickerDepth depth, bool cram) onChoose) {
  return MaterialApp(
    theme: alixDark(),
    home: Scaffold(
      body: PickerDepthSheet(
        selected: PickerDepth.recall,
        canRecognize: true,
        onChoose: onChoose,
      ),
    ),
  );
}

void main() {
  testWidgets('a depth chosen with the cram switch untouched is a plain '
      'session', (tester) async {
    final choices = <(PickerDepth, bool)>[];
    await tester.pumpWidget(_sheet((depth, cram) => choices.add((depth, cram))));

    await tester.tap(find.text('Recall'));

    expect(choices, [(PickerDepth.recall, false)]);
  });

  testWidgets('the cram switch rides along with whichever depth is chosen', (
    tester,
  ) async {
    final choices = <(PickerDepth, bool)>[];
    await tester.pumpWidget(_sheet((depth, cram) => choices.add((depth, cram))));

    await tester.tap(find.text('Cram'));
    await tester.pump();
    await tester.tap(find.text('Reconstruct'));

    expect(choices, [(PickerDepth.reconstruct, true)]);
  });

  testWidgets('switching cram on says what to do next', (tester) async {
    await tester.pumpWidget(_sheet((_, _) {}));
    expect(find.text('now pick a depth to start'), findsNothing);

    await tester.tap(find.text('Cram'));
    await tester.pump();

    expect(find.text('now pick a depth to start'), findsOneWidget);
    expect(
      tester.getTopLeft(find.text('Cram')).dy,
      lessThan(tester.getTopLeft(find.text('Recognize')).dy),
      reason: 'the modifier sits above the launches it modifies',
    );
  });

  testWidgets('every opening of the sheet starts with cram off', (tester) async {
    final choices = <(PickerDepth, bool)>[];
    await tester.pumpWidget(_sheet((depth, cram) => choices.add((depth, cram))));
    await tester.tap(find.text('Cram'));
    await tester.pump();

    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pumpWidget(_sheet((depth, cram) => choices.add((depth, cram))));
    await tester.tap(find.text('Recall'));

    expect(choices, [(PickerDepth.recall, false)]);
  });
}
