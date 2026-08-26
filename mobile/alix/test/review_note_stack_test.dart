import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/theme.dart';

ReviewNoteModel _note(String text, {ReviewBadge? badge}) => ReviewNoteModel(
  badge: badge,
  units: [
    ReviewSentenceModel(
      text: text,
      runs: [
        InlineRunModel(text: text, bold: false, italic: false, code: false),
      ],
    ),
  ],
);

Widget _card(List<ReviewNoteModel> notes, TextEditingController attempt) {
  final card = ReviewCardModel(
    front: 'Question',
    frontRuns: const [],
    context: const [],
    contextLeads: false,
    contextRuns: const [],
    contextUnits: const [],
    back: const ['Answer'],
    backRuns: const [[]],
    backUnits: const [],
    answerSteps: const [ReviewAnswerLineModel(backFrom: 0, backTo: 1)],
    reshaped: false,
    note: notes,
    images: const [],
    imagesBack: const [],
  );
  final state = ReviewStateModel(
    card: card,
    mode: ReviewMode.flip,
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
  );
  return MaterialApp(
    theme: alixDark(),
    home: Scaffold(
      body: ReviewCardView(
        state: state,
        revealed: true,
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
      ),
    ),
  );
}

Finder get _noteBoxes => find.byWidgetPredicate(
  (widget) => widget is Container && widget.constraints?.maxWidth == 600,
);

Color _boxColour(WidgetTester tester) {
  final box = tester.widget<Container>(_noteBoxes.first);
  return (box.decoration! as BoxDecoration).color!;
}

void main() {
  testWidgets('each note renders in its own box and keeps authored order', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);

    await tester.pumpWidget(
      _card([
        _note('First.', badge: ReviewBadge.warning),
        _note('Second.'),
      ], attempt),
    );

    expect(find.text('First.'), findsOneWidget);
    expect(find.text('Second.'), findsOneWidget);
    expect(_noteBoxes, findsNWidgets(2));
  });

  testWidgets('every badge names itself and tints its own box', (tester) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    final tokens = alixDark().alix;
    final accents = {
      ReviewBadge.note: tokens.bolt,
      ReviewBadge.tip: tokens.good,
      ReviewBadge.important: tokens.bolt,
      ReviewBadge.warning: tokens.warn,
      ReviewBadge.caution: tokens.again,
    };

    for (final badge in ReviewBadge.values) {
      await tester.pumpWidget(_card([_note('Body.', badge: badge)], attempt));
      final name = badge.name.toUpperCase();
      expect(find.text(name), findsOneWidget, reason: '$badge names its chip');
      expect(
        _boxColour(tester),
        accents[badge]!.withValues(alpha: 0.12),
        reason: '$badge tints its box with its own accent',
      );
    }
  });

  testWidgets('a badgeless note keeps the plain note ground and no chip', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    final tokens = alixDark().alix;

    await tester.pumpWidget(_card([_note('A table column.')], attempt));

    for (final badge in ReviewBadge.values) {
      expect(
        find.text(badge.name.toUpperCase()),
        findsNothing,
        reason: 'no badge, no chip',
      );
    }
    expect(_boxColour(tester), tokens.noteBorder.withValues(alpha: 0.12));
  });
}
