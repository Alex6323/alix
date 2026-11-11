import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

import 'review_cloze_reveal_test.dart' show openCard;

String _deck({required bool topic, required bool hint}) =>
    '${topic ? '# Ownership\nWho frees a value, and when.\n\n' : ''}'
    '## What does `Rc<T>` count?\n'
    '${hint ? '> [!TIP]\n> Think about sharing.\n\n---\n' : ''}'
    'The number of owners of the shared value.\n'
    '<!-- id: card-slots -->\n';

Future<void> _pump(
  WidgetTester tester,
  ReviewCardModel card, {
  bool revealed = false,
  bool tutor = false,
  TextScaler? textScaler,
}) async {
  final attempt = TextEditingController();
  addTearDown(attempt.dispose);
  await tester.pumpWidget(
    MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: alixDark(),
      builder: textScaler == null
          ? null
          : (context, child) => MediaQuery(
              data: MediaQuery.of(context).copyWith(textScaler: textScaler),
              child: child!,
            ),
      home: Scaffold(
        body: ReviewCardView(
          state: ReviewStateModel(
            card: card,
            mode: ReviewMode.flip,
            depth: ReviewDepth.recall,
            input: ReviewInput.type,
            introducing: false,
            choices: null,
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
          ),
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
          serverLive: tutor,
          tutorCard: tutor
              ? ReviewTutorCardModel(
                  id: 'card-slots',
                  deckId: 'deck-d',
                  subject: 's',
                  front: 'f',
                  back: const ['b'],
                  line: 1,
                )
              : null,
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
          onOpenTopic: (_) {},
          onOpenHint: (_) {},
        ),
      ),
    ),
  );
  await tester.pump();
}

void main() {
  setUpAll(() async => RustLib.init());

  testWidgets('large text keeps Context and Hint outside the centred action', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3.0;
    addTearDown(tester.view.reset);
    await _pump(
      tester,
      openCard(_deck(topic: true, hint: true)),
      textScaler: const TextScaler.linear(2),
    );

    Rect chip(String label) => tester.getRect(
      find.ancestor(of: find.text(label), matching: find.byType(ReviewChip)),
    );
    final context = chip('Context');
    final reveal = chip('Reveal');
    final hint = chip('Hint');
    expect(
      !context.overlaps(reveal) &&
          !reveal.overlaps(hint) &&
          !context.overlaps(hint) &&
          context.left >= 0 &&
          hint.right <= 360,
      isTrue,
      reason:
          'Context $context, Reveal $reveal, and Hint $hint must be separate and on screen',
    );
  });

  testWidgets(
    'large text keeps each side action at least 44 logical pixels high',
    (tester) async {
      tester.view.physicalSize = const Size(1080, 2280);
      tester.view.devicePixelRatio = 3.0;
      addTearDown(tester.view.reset);
      await _pump(
        tester,
        openCard(_deck(topic: true, hint: true)),
        textScaler: const TextScaler.linear(2),
      );

      Rect tapTarget(String label) => tester.getRect(
        find.ancestor(of: find.text(label), matching: find.byType(InkWell)),
      );
      final targets = {
        for (final label in ['Context', 'Hint']) label: tapTarget(label),
      };
      expect(
        targets.values.every((target) => target.height >= 44),
        isTrue,
        reason:
            'side tap targets $targets must each remain at least 44 logical pixels high',
      );
    },
  );
}
