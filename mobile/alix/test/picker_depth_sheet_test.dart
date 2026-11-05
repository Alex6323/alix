import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/picker_widgets.dart';
import 'package:alix_mobile/theme.dart';

Widget _sheet(
  ValueChanged<PickerLaunch> onChoose, {
  VoidCallback? onWalk,
  PickerExamOffer? exam,
}) {
  return MaterialApp(
    theme: alixDark(),
    home: Scaffold(
      body: PickerDepthSheet(
        canRecognize: true,
        onChoose: onChoose,
        onWalk: onWalk,
        exam: exam,
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

  testWidgets(
    'the Walk row sits below the depths, describes itself, and launches a '
    'walk without a depth',
    (tester) async {
      final choices = <PickerLaunch>[];
      var walks = 0;
      await tester.pumpWidget(_sheet(choices.add, onWalk: () => walks++));

      final walk = find.widgetWithText(InkWell, 'Walk');
      expect(walk, findsOneWidget, reason: 'the row is offered');
      expect(
        find.descendant(
          of: walk,
          matching: find.text('every card once, unscheduled'),
        ),
        findsOneWidget,
        reason: 'the row says what a walk is, like its siblings',
      );
      expect(
        tester.getTopLeft(walk).dy,
        greaterThan(tester.getTopLeft(find.text('Reconstruct')).dy),
        reason: 'the walk sits below the depth launches',
      );
      await tester.tap(walk);

      expect((walks, choices.length), (1, 0), reason: 'one walk, no depth');
    },
  );

  testWidgets('a hairline separates the walk from the switches and depths it '
      'ignores', (tester) async {
    await tester.pumpWidget(_sheet((_) {}, onWalk: () {}));

    final divider = find.byType(Divider);
    expect(divider, findsOneWidget, reason: 'one separator in the sheet');
    expect(
      tester.widget<Divider>(divider).thickness,
      0,
      reason: 'a Flutter hairline is exactly one device pixel',
    );
    final line = tester.getCenter(divider).dy;
    expect(
      (
        line >
            tester
                .getBottomLeft(find.widgetWithText(InkWell, 'Reconstruct'))
                .dy,
        line < tester.getTopLeft(find.widgetWithText(InkWell, 'Walk')).dy,
      ),
      (true, true),
      reason: 'separator at y=$line lies between Reconstruct and Walk',
    );
  });

  testWidgets('the hairline is short, centered, and equally far from both '
      'rows', (tester) async {
    await tester.pumpWidget(_sheet((_) {}, onWalk: () {}));

    Rect frame(String label) => tester.getRect(
      find.descendant(
        of: find.widgetWithText(InkWell, label),
        matching: find.byWidgetPredicate(
          (widget) =>
              widget is Container &&
              widget.decoration is BoxDecoration &&
              (widget.decoration! as BoxDecoration).border != null,
        ),
      ),
    );
    final above = frame('Reconstruct');
    final below = frame('Walk');
    final line = tester.getRect(find.byType(Divider));
    final gapAbove = line.center.dy - above.bottom;
    final gapBelow = below.top - line.center.dy;
    expect(
      (gapAbove - gapBelow).abs(),
      lessThan(0.5),
      reason: 'gap above $gapAbove vs below $gapBelow',
    );
    expect(
      line.width,
      lessThan(above.width / 2),
      reason: 'line ${line.width} wide under a ${above.width} row',
    );
    expect(
      (line.center.dx - above.center.dx).abs(),
      lessThan(0.5),
      reason: 'line centered at ${line.center.dx}, row at ${above.center.dx}',
    );
  });

  testWidgets('without a walk launcher the sheet has no Walk row', (
    tester,
  ) async {
    await tester.pumpWidget(_sheet((_) {}));
    expect(find.text('Walk'), findsNothing);
    expect(find.byType(Divider), findsNothing, reason: 'nothing to separate');
  });

  testWidgets('every launch is a bordered row with a chevron, like the '
      "picker's deck rows", (tester) async {
    await tester.pumpWidget(_sheet((_) {}, onWalk: () {}));

    for (final launch in ['Recognize', 'Recall', 'Reconstruct', 'Walk']) {
      final row = find.widgetWithText(InkWell, launch);
      expect(row, findsOneWidget, reason: '$launch is one tappable row');
      final framed = find.descendant(
        of: row,
        matching: find.byWidgetPredicate(
          (widget) =>
              widget is Container &&
              widget.decoration is BoxDecoration &&
              (widget.decoration! as BoxDecoration).border != null,
        ),
      );
      expect(framed, findsOneWidget, reason: '$launch carries a border');
      expect(
        find.descendant(of: row, matching: find.byIcon(Icons.chevron_right)),
        findsOneWidget,
        reason: '$launch shows a chevron',
      );
    }
  });

  PickerExamOffer offer({
    bool locked = false,
    bool serverLive = true,
    bool due = false,
    bool filterable = false,
    PickerExamAction action = PickerExamAction.generate,
    String? estimate,
    ValueChanged<bool>? onStart,
  }) => PickerExamOffer(
    locked: locked,
    serverLive: serverLive,
    due: due,
    filterable: filterable,
    action: action,
    estimate: estimate,
    onStart: onStart ?? (_) {},
  );

  BoxDecoration examFrame(
    WidgetTester tester, {
    String label = 'Generate exam',
  }) {
    final framed = find.descendant(
      of: find.widgetWithText(InkWell, label),
      matching: find.byWidgetPredicate(
        (widget) =>
            widget is Container &&
            widget.decoration is BoxDecoration &&
            (widget.decoration! as BoxDecoration).border != null,
      ),
    );
    return (tester.widget<Container>(framed)).decoration! as BoxDecoration;
  }

  Color? fill(WidgetTester tester) => examFrame(tester).color;

  testWidgets('without an exam offer the sheet has no Exam row or filter', (
    tester,
  ) async {
    await tester.pumpWidget(_sheet((_) {}, onWalk: () {}));
    expect(find.text('Generate exam'), findsNothing);
    expect(find.text('Only what needs work'), findsNothing);
  });

  testWidgets(
    'the Generate exam row sits below Walk and starts a full sitting',
    (tester) async {
      final starts = <bool>[];
      await tester.pumpWidget(
        _sheet(
          (_) {},
          onWalk: () {},
          exam: offer(onStart: starts.add),
        ),
      );

      expect(
        tester.getTopLeft(find.text('Generate exam')).dy,
        greaterThan(tester.getTopLeft(find.text('Walk')).dy),
      );
      await tester.tap(find.text('Generate exam'));
      expect(starts, [false]);
    },
  );

  testWidgets('a locked exam is disabled and says so', (tester) async {
    final starts = <bool>[];
    await tester.pumpWidget(
      _sheet((_) {}, exam: offer(locked: true, due: true, onStart: starts.add)),
    );

    await tester.tap(find.text('Generate exam'));
    expect(starts, isEmpty);
    expect(find.text('locked'), findsOneWidget);
    expect(fill(tester), isNull, reason: 'a locked exam is never filled');
  });

  testWidgets('an exam with no live paired server is disabled and says so', (
    tester,
  ) async {
    final starts = <bool>[];
    await tester.pumpWidget(
      _sheet((_) {}, exam: offer(serverLive: false, onStart: starts.add)),
    );

    await tester.tap(find.text('Generate exam'));
    expect(starts, isEmpty);
    expect(find.text('needs a paired desktop'), findsOneWidget);
  });

  testWidgets('the Generate exam row is filled only when due and unlocked', (
    tester,
  ) async {
    await tester.pumpWidget(_sheet((_) {}, exam: offer(due: true)));
    expect(fill(tester), isNotNull);
    expect(
      (examFrame(tester).border! as Border).top.color,
      Colors.transparent,
      reason: 'a filled row carries no highlighted border',
    );

    await tester.pumpWidget(_sheet((_) {}, exam: offer(due: false)));
    expect(fill(tester), isNull);
  });

  testWidgets('the filter switch is always shown, on by default, and off and '
      'disabled when it would select nothing', (tester) async {
    SwitchListTile filterSwitch() =>
        tester.widget<SwitchListTile>(find.byType(SwitchListTile).last);

    final idle = <bool>[];
    await tester.pumpWidget(_sheet((_) {}, exam: offer(onStart: idle.add)));
    expect(find.text('Only what needs work'), findsOneWidget);
    expect(filterSwitch().value, isFalse);
    expect(filterSwitch().onChanged, isNull);
    await tester.tap(find.text('Generate exam'));
    expect(idle, [false], reason: 'nothing to filter: a full sitting');

    final starts = <bool>[];
    await tester.pumpWidget(
      _sheet((_) {}, exam: offer(filterable: true, onStart: starts.add)),
    );
    expect(filterSwitch().value, isTrue);
    await tester.tap(find.text('Generate exam'));
    await tester.tap(find.text('Only what needs work'));
    await tester.pump();
    await tester.tap(find.text('Generate exam'));
    expect(starts, [true, false]);
  });

  testWidgets('Preparing shows the estimate and cannot be started again', (
    tester,
  ) async {
    final starts = <bool>[];
    await tester.pumpWidget(
      _sheet(
        (_) {},
        exam: offer(
          action: PickerExamAction.preparing,
          estimate: 'about 20 to 30 minutes',
          onStart: starts.add,
        ),
      ),
    );

    expect(find.text('Preparing'), findsOneWidget);
    expect(find.text('about 20 to 30 minutes'), findsOneWidget);
    await tester.tap(find.text('Preparing'));
    expect(starts, isEmpty);
  });

  testWidgets('Take exam opens the ready sitting and is filled', (
    tester,
  ) async {
    final starts = <bool>[];
    await tester.pumpWidget(
      _sheet(
        (_) {},
        exam: offer(action: PickerExamAction.take, onStart: starts.add),
      ),
    );

    expect(find.text('Take exam'), findsOneWidget);
    expect(examFrame(tester, label: 'Take exam').color, isNotNull);
    await tester.tap(find.text('Take exam'));
    expect(starts, [false]);
  });
}
