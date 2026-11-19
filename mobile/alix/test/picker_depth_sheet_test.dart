import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/picker_widgets.dart';
import 'package:alix_mobile/theme.dart';

Widget _sheet(
  ValueChanged<PickerLaunch> onChoose, {
  ValueChanged<bool>? onWalk,
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
    expect(find.text('now pick a depth or walk'), findsNothing);

    await tester.tap(find.text('Cram'));
    await tester.pump();

    expect(find.text('now pick a depth or walk'), findsOneWidget);
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
      await tester.pumpWidget(_sheet(choices.add, onWalk: (_) => walks++));

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

  testWidgets('with Cram on, Walk says it resumes and launches a cram walk', (
    tester,
  ) async {
    final walks = <bool>[];
    await tester.pumpWidget(_sheet((_) {}, onWalk: walks.add));
    await tester.tap(find.text('Cram'));
    await tester.pump();

    final walk = find.widgetWithText(InkWell, 'Walk');
    expect(
      find.descendant(
        of: walk,
        matching: find.text('every card, from where you stopped'),
      ),
      findsOneWidget,
      reason: 'the row says a cram walk resumes',
    );
    await tester.tap(walk);
    expect(walks, [true], reason: 'the walk launches with cram');
  });

  testWidgets('a hairline separates the walk from the switches and depths it '
      'ignores', (tester) async {
    await tester.pumpWidget(_sheet((_) {}, onWalk: (_) {}));

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
    await tester.pumpWidget(_sheet((_) {}, onWalk: (_) {}));

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
    await tester.pumpWidget(_sheet((_) {}, onWalk: (_) {}));

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
    bool failed = false,
    DateTime? preparingSince,
    ValueChanged<bool>? onStart,
    VoidCallback? onCancel,
    VoidCallback? onDiscard,
    DateTime Function()? now,
  }) => PickerExamOffer(
    locked: locked,
    serverLive: serverLive,
    due: due,
    filterable: filterable,
    action: action,
    failed: failed,
    preparingSince: preparingSince,
    onStart: onStart ?? (_) {},
    onCancel: onCancel,
    onDiscard: onDiscard,
    now: now ?? DateTime.now,
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
    await tester.pumpWidget(_sheet((_) {}, onWalk: (_) {}));
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
          onWalk: (_) {},
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

  testWidgets(
    'a preparing exam keeps its Generate exam row, ticks its timer, and a tap '
    'asks to cancel instead of starting again',
    (tester) async {
      final starts = <bool>[];
      var cancels = 0;
      final since = DateTime(2026, 11, 11, 1, 0, 0);
      var now = since.add(const Duration(seconds: 20));
      await tester.pumpWidget(
        _sheet(
          (_) {},
          exam: offer(
            action: PickerExamAction.preparing,
            preparingSince: since,
            now: () => now,
            onStart: starts.add,
            onCancel: () => cancels++,
          ),
        ),
      );

      expect(find.text('Generate exam'), findsOneWidget, reason: 'step 1: label kept');
      expect(find.text('preparing… 20s'), findsOneWidget, reason: 'step 1: timer');
      expect(fill(tester), isNull, reason: 'step 1: a preparing row is not filled');

      now = since.add(const Duration(seconds: 81));
      await tester.pump(const Duration(seconds: 1));
      expect(
        find.text('preparing… 1m 21s'),
        findsOneWidget,
        reason: 'step 2: the row ticks every second',
      );

      await tester.tap(find.text('Generate exam'));
      expect(starts, isEmpty, reason: 'step 3: no second start');
      expect(cancels, 1, reason: 'step 3: the tap asks to cancel');
    },
  );

  testWidgets('a failed preparation offers to generate again and says why', (
    tester,
  ) async {
    final starts = <bool>[];
    await tester.pumpWidget(
      _sheet((_) {}, exam: offer(failed: true, onStart: starts.add)),
    );

    expect(find.text('the last preparation failed'), findsOneWidget);
    await tester.tap(find.text('Generate exam'));
    expect(starts, [false]);
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
    final context = tester.element(find.text('Take exam'));
    expect(
      examFrame(tester, label: 'Take exam').color,
      Theme.of(context).colorScheme.onSurface,
      reason: 'a ready exam is filled with the theme ink, not the accent',
    );
    await tester.tap(find.text('Take exam'));
    expect(starts, [false]);
  });

  testWidgets(
    'Take exam and Show result work with the desktop unreachable, Generate '
    'exam does not',
    (tester) async {
      for (final (action, label) in [
        (PickerExamAction.take, 'Take exam'),
        (PickerExamAction.result, 'Show result'),
        (PickerExamAction.generate, 'Generate exam'),
      ]) {
        final starts = <bool>[];
        await tester.pumpWidget(
          _sheet(
            (_) {},
            exam: offer(serverLive: false, action: action, onStart: starts.add),
          ),
        );
        await tester.tap(find.text(label));
        final expected = action == PickerExamAction.generate ? 0 : 1;
        expect(
          starts.length,
          expected,
          reason: '$action without a desktop: taps on "$label" reached onStart ${starts.length} times, expected $expected',
        );
      }
    },
  );

  testWidgets('a locked exam stays disabled for every action', (tester) async {
    for (final action in PickerExamAction.values) {
      final starts = <bool>[];
      await tester.pumpWidget(
        _sheet(
          (_) {},
          exam: offer(locked: true, action: action, onStart: starts.add, onCancel: starts.isEmpty ? () => starts.add(true) : null),
        ),
      );
      await tester.tap(find.text('locked').hitTestable(), warnIfMissed: false);
      expect(find.text('locked'), findsOneWidget, reason: '$action: the row says locked');
      expect(starts, isEmpty, reason: '$action: a locked row starts or cancels nothing, got $starts');
    }
  });

  testWidgets('the hints of Take exam and Show result do not ask for a desktop to take or show', (tester) async {
    await tester.pumpWidget(
      _sheet((_) {}, exam: offer(serverLive: false, action: PickerExamAction.take)),
    );
    expect(find.text('grading needs your paired desktop'), findsOneWidget, reason: 'take without a desktop: hint says grading needs it');
    expect(find.text('needs a paired desktop'), findsNothing, reason: 'take without a desktop: no desktop requirement on taking');

    await tester.pumpWidget(
      _sheet((_) {}, exam: offer(serverLive: false, action: PickerExamAction.result)),
    );
    expect(find.text('your graded exam'), findsOneWidget, reason: 'result without a desktop: hint');
    expect(find.text('needs a paired desktop'), findsNothing, reason: 'result without a desktop: no desktop requirement');
  });

  testWidgets('a fetching exam shows the preparing row and a tap does nothing', (tester) async {
    final starts = <bool>[];
    var cancels = 0;
    final since = DateTime(2026, 11, 11, 1, 0, 0);
    await tester.pumpWidget(
      _sheet(
        (_) {},
        exam: offer(
          action: PickerExamAction.fetching,
          preparingSince: since,
          now: () => since.add(const Duration(seconds: 20)),
          onStart: starts.add,
          onCancel: () => cancels++,
        ),
      ),
    );
    expect(find.text('Generate exam'), findsOneWidget, reason: 'fetching: label kept');
    expect(find.text('preparing… 20s'), findsOneWidget, reason: 'fetching: same hint as preparing');

    await tester.tap(find.text('Generate exam'));
    expect(starts, isEmpty, reason: 'fetching: a tap starts nothing, got $starts');
    expect(cancels, 0, reason: 'fetching: a tap cancels nothing, got $cancels');
  });

  testWidgets('the exam row has a menu with Discard exam only when the offer carries onDiscard', (tester) async {
    await tester.pumpWidget(_sheet((_) {}, exam: offer()));
    expect(find.byIcon(Icons.more_vert), findsNothing, reason: 'no onDiscard: no menu button');

    await tester.pumpWidget(_sheet((_) {}, exam: offer(onDiscard: () {})));
    expect(find.byIcon(Icons.more_vert), findsOneWidget, reason: 'onDiscard set: one menu button');
    expect(find.text('Discard exam'), findsNothing, reason: 'the entry shows only once the menu opens');
    await tester.tap(find.byIcon(Icons.more_vert));
    await tester.pumpAndSettle();
    expect(find.text('Discard exam'), findsOneWidget, reason: 'menu open: the single entry');
  });

  testWidgets('choosing Discard exam calls onDiscard once and not the row tap, also on a disabled row', (tester) async {
    for (final (live, action) in [
      (true, PickerExamAction.generate),
      (true, PickerExamAction.preparing),
      (false, PickerExamAction.take),
      (false, PickerExamAction.generate),
    ]) {
      final starts = <bool>[];
      var cancels = 0;
      var discards = 0;
      await tester.pumpWidget(
        _sheet(
          (_) {},
          exam: offer(
            serverLive: live,
            action: action,
            onStart: starts.add,
            onCancel: () => cancels++,
            onDiscard: () => discards++,
          ),
        ),
      );
      await tester.tap(find.byIcon(Icons.more_vert));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Discard exam'));
      await tester.pumpAndSettle();
      expect(discards, 1, reason: '$action live=$live: onDiscard calls $discards, expected 1');
      expect(starts, isEmpty, reason: '$action live=$live: the row start was not triggered, got $starts');
      expect(cancels, 0, reason: '$action live=$live: the row cancel was not triggered, got $cancels');
    }
  });

  testWidgets('the exam row with a menu lays out on a phone for every action', (tester) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);
    for (final action in PickerExamAction.values) {
      await tester.pumpWidget(
        _sheet(
          (_) {},
          onWalk: (_) {},
          exam: offer(action: action, preparingSince: DateTime(2026, 11, 11), onDiscard: () {}),
        ),
      );
      expect(tester.takeException(), isNull, reason: '$action at 360x760: the sheet lays out without an exception');
    }
  });
}
