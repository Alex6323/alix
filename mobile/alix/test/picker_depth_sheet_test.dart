import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/picker_widgets.dart';
import 'package:alix_mobile/theme.dart';

Widget _sheet(ValueChanged<PickerLaunch> onChoose) {
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
  testWidgets('a depth chosen with both switches untouched is a plain '
      'session', (tester) async {
    final choices = <PickerLaunch>[];
    await tester.pumpWidget(_sheet(choices.add));

    await tester.tap(find.text('Recall'));

    expect(choices, [
      (depth: PickerDepth.recall, cram: false, skipIntroduction: false),
    ]);
  });

  testWidgets('the cram switch rides along with whichever depth is chosen', (
    tester,
  ) async {
    final choices = <PickerLaunch>[];
    await tester.pumpWidget(_sheet(choices.add));

    await tester.tap(find.text('Cram'));
    await tester.pump();
    await tester.tap(find.text('Reconstruct'));

    expect(choices, [
      (depth: PickerDepth.reconstruct, cram: true, skipIntroduction: false),
    ]);
  });

  testWidgets('the skip-introduction switch rides along on its own or with '
      'cram', (tester) async {
    final choices = <PickerLaunch>[];
    await tester.pumpWidget(_sheet(choices.add));

    await tester.tap(find.text('Skip introduction'));
    await tester.pump();
    await tester.tap(find.text('Recall'));
    await tester.tap(find.text('Cram'));
    await tester.pump();
    await tester.tap(find.text('Recall'));

    expect(choices, [
      (depth: PickerDepth.recall, cram: false, skipIntroduction: true),
      (depth: PickerDepth.recall, cram: true, skipIntroduction: true),
    ]);
  });

  testWidgets('switching cram on says what to do next', (tester) async {
    await tester.pumpWidget(_sheet((_) {}));
    expect(find.text('now pick a depth to start'), findsNothing);

    await tester.tap(find.text('Cram'));
    await tester.pump();

    expect(find.text('now pick a depth to start'), findsOneWidget);
    expect(
      tester.getTopLeft(find.text('Cram')).dy,
      lessThan(tester.getTopLeft(find.text('Skip introduction')).dy),
      reason: 'cram first, then the introduction switch',
    );
    expect(
      tester.getTopLeft(find.text('Skip introduction')).dy,
      lessThan(tester.getTopLeft(find.text('Recognize')).dy),
      reason: 'the modifiers sit above the launches they modify',
    );
  });

  testWidgets('every opening of the sheet starts with both switches off', (
    tester,
  ) async {
    final choices = <PickerLaunch>[];
    await tester.pumpWidget(_sheet(choices.add));
    await tester.tap(find.text('Cram'));
    await tester.tap(find.text('Skip introduction'));
    await tester.pump();

    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pumpWidget(_sheet(choices.add));
    await tester.tap(find.text('Recall'));

    expect(choices, [
      (depth: PickerDepth.recall, cram: false, skipIntroduction: false),
    ]);
  });
}
