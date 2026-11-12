import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

import 'review_cloze_reveal_test.dart' show openCard;

const _deck = '''
# Ownership
Who frees a value, and when.

## What does `Rc<T>` count?
The number of owners of the shared value.
> [!NOTE]
> It counts each live owning pointer.
<!-- id: card-slots -->
''';

Future<void> _loadFonts() async {
  Future<ByteData> bytes(String path) async =>
      (await File(path).readAsBytes()).buffer.asByteData();
  final sans = FontLoader('IBM Plex Sans');
  for (final weight in const ['400', '500', '600', '700']) {
    sans.addFont(bytes('assets/fonts/ibm-plex-sans-$weight.ttf'));
  }
  final mono = FontLoader('IBM Plex Mono');
  for (final weight in const ['400', '600']) {
    mono.addFont(bytes('assets/fonts/ibm-plex-mono-$weight.ttf'));
  }
  await sans.load();
  await mono.load();
}

Future<void> _pump(WidgetTester tester, {required bool tutor}) async {
  final attempt = TextEditingController();
  addTearDown(attempt.dispose);
  await tester.pumpWidget(
    MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: alixDark(),
      home: Scaffold(
        body: ReviewCardView(
          state: ReviewStateModel(
            card: openCard(_deck),
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
          serverLive: tutor,
          tutorCard: tutor
              ? ReviewTutorCardModel(
                  id: 'card-slots',
                  deckId: 'deck-d',
                  subject: 's',
                  front: 'f',
                  back: ['b'],
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
        ),
      ),
    ),
  );
  await tester.pump();
}

void main() {
  setUpAll(() async {
    await RustLib.init();
    await _loadFonts();
  });

  testWidgets(
    'Ask tutor does not move an answered card body or its answer-note split',
    (tester) async {
      tester.view.physicalSize = const Size(360, 760);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      ({Rect body, Rect answer, Rect note}) geometry() => (
        body: tester.getRect(find.byKey(const ValueKey('card-body'))),
        answer: tester.getRect(find.byKey(const ValueKey('answer-region'))),
        note: tester.getRect(find.byKey(const ValueKey('note-region'))),
      );

      await _pump(tester, tutor: false);
      final withoutTutor = geometry();
      await _pump(tester, tutor: true);
      final withTutor = geometry();

      expect(
        withTutor,
        withoutTutor,
        reason:
            'adding the Ask affordance must not take room from the card body; '
            'without tutor $withoutTutor, with tutor $withTutor',
      );
    },
  );
}
