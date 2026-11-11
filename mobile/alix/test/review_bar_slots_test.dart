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
  double textScale = 1,
}) async {
  final attempt = TextEditingController();
  addTearDown(attempt.dispose);
  await tester.pumpWidget(
    MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: alixDark(),
      builder: (context, child) => MediaQuery(
        data: MediaQuery.of(context).copyWith(textScaler: TextScaler.linear(textScale)),
        child: child!,
      ),
      home: Scaffold(
        body: ReviewCardView(
          state: ReviewStateModel(
            card: card, mode: ReviewMode.flip, depth: ReviewDepth.recall,
            input: ReviewInput.type, introducing: false, choices: null,
            finished: false, remaining: 1, reviews: 0, passed: 0, failed: 0,
            introduced: 0, partial: 0, canRestart: false, dueLeft: 1, newLeft: 0,
          ),
          revealed: revealed, revealedLines: 0, choice: null, multiChoice: null,
          multiSelected: const {}, checkFeedback: null, typelineChecked: const [],
          tickedKeypoints: const {}, sketch: Sketch(),
          onSketchBegin: (_, _) {}, onSketchExtend: (_) {}, onSketchEnd: () {},
          onSketchTool: (_) {}, onSketchUndo: () {}, onSketchClear: () {},
          attemptOpen: false, attemptController: attempt, typedControllers: const [],
          serverLive: tutor,
          tutorCard: tutor
              ? ReviewTutorCardModel(
                  id: 'card-slots', deckId: 'deck-d', subject: 's', front: 'f',
                  back: const ['b'], line: 1)
              : null,
          verdictGrade: ReviewGrade.pass,
          onChoose: (_) {}, onToggleChoice: (_) {}, onSubmitChoices: () {},
          onCheck: (_) {}, onOpenAttempt: () {}, onToggleKeypoint: (_) {},
          onReveal: () {}, onRevealNextLine: () {}, onIntroduce: () {},
          onGrade: (_) {}, onOpenTutor: (_) {}, onOpenTopic: (_) {},
          onOpenHint: (_) {},
        ),
      ),
    ),
  );
  await tester.pump();
}

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

void main() {
  setUpAll(() async {
    await RustLib.init();
    await _loadFonts();
  });

  testWidgets('Reveal holds the centre whichever of Context and Hint the card has', (tester) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3.0;
    addTearDown(tester.view.reset);
    final centre = tester.view.physicalSize.width / tester.view.devicePixelRatio / 2;
    for (final (topic, hint, scale) in [
      for (final scale in [1.0, 2.0])
        for (final (t, h) in [(true, true), (true, false), (false, true), (false, false)]) (t, h, scale),
    ]) {
      final label = 'topic $topic, hint $hint, text x$scale';
      await _pump(tester, openCard(_deck(topic: topic, hint: hint)), textScale: scale);
      final reveal = tester.getCenter(find.text('Reveal'));
      expect((reveal.dx - centre).abs(), lessThanOrEqualTo(1),
          reason: '$label: Reveal ${reveal.dx} sits on the centre $centre');
      expect(find.text('Context'), topic ? findsOneWidget : findsNothing,
          reason: '$label: an absent Context leaves its slot blank');
      expect(find.text('Hint'), hint ? findsOneWidget : findsNothing,
          reason: '$label: an absent Hint leaves its slot blank');
      if (topic) {
        expect(tester.getRect(find.text('Context')).left, lessThan(centre / 2),
            reason: '$label: Context sits at the left edge');
      }
      if (hint) {
        expect(tester.getRect(find.text('Hint')).right, greaterThan(centre * 1.5),
            reason: '$label: Hint sits at the right edge');
      }
    }
  });

  testWidgets('after answering, equal grade buttons hold the centre and Ask is a glyph at the right', (tester) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3.0;
    addTearDown(tester.view.reset);
    final width = tester.view.physicalSize.width / tester.view.devicePixelRatio;
    for (final (tutor, scale) in [(true, 1.0), (false, 1.0), (true, 2.0), (false, 2.0)]) {
      final label = 'tutor $tutor, text x$scale';
      await _pump(tester, openCard(_deck(topic: true, hint: true)), revealed: true, tutor: tutor, textScale: scale);
      final rects = [
        for (final grade in ['Missed it', 'Partly', 'Got it'])
          tester.getRect(find.ancestor(of: find.text(grade), matching: find.byType(ReviewChip))),
      ];
      for (final rect in rects.skip(1)) {
        expect((rect.width - rects.first.width).abs(), lessThanOrEqualTo(0.5),
            reason: '$label: grade buttons are equal width (${rects.map((r) => r.width)})');
      }
      final group = (rects.first.left + rects.last.right) / 2;
      expect((group - width / 2).abs(), lessThanOrEqualTo(1),
          reason: '$label: the grade group centre $group sits on the screen centre');
      expect(find.text('Ask'), findsNothing, reason: '$label: no Ask text chip');
      final ask = find.byKey(const ValueKey('ask-glyph'));
      expect(ask, tutor ? findsOneWidget : findsNothing,
          reason: '$label: the Ask glyph shows only with a live tutor');
      for (final rect in rects) {
        expect(rect.left >= 0 && rect.right <= width, isTrue,
            reason: '$label: grade button $rect stays on screen');
      }
      if (!tutor) {
        final height = tester.view.physicalSize.height / tester.view.devicePixelRatio;
        expect((height - 12 - rects.last.bottom).abs(), lessThanOrEqualTo(1),
            reason: '$label: with no side chips the bar is only the grade row, '
                'no empty second line (grade bottom ${rects.last.bottom})');
      }
      if (tutor) {
        final glyph = tester.getRect(ask);
        expect(rects.any((rect) => rect.overlaps(glyph)), isFalse,
            reason: '$label: the Ask glyph $glyph overlaps no grade button');
        expect(glyph.right, lessThanOrEqualTo(width),
            reason: '$label: the Ask glyph stays on screen');
        if (scale == 1.0) {
          expect((glyph.center.dy - rects.last.center.dy).abs(), lessThanOrEqualTo(1),
              reason: '$label: at normal text the Ask glyph shares the grade row');
        }
      }
    }
  });

  testWidgets('Ask tutor keeps a 44 logical pixel tap target', (tester) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3.0;
    addTearDown(tester.view.reset);
    await _pump(
      tester,
      openCard(_deck(topic: true, hint: true)),
      revealed: true,
      tutor: true,
    );

    final target = tester.getRect(find.byKey(const ValueKey('ask-glyph')));
    expect(
      target.width >= 44 && target.height >= 44,
      isTrue,
      reason: 'Ask tutor tap target $target must be at least 44 by 44',
    );
  });
}
