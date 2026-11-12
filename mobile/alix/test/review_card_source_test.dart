import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/theme.dart';

InlineRunModel _run(String text) =>
    InlineRunModel(text: text, bold: false, italic: false, code: false);

ReviewCitationModel _citation(String line) => ReviewCitationModel(
  locator: 'src:source.rs:7',
  excerpt: ReviewExcerptModel(
    path: 'source.rs',
    lines: [ReviewExcerptLineModel(number: 7, text: line)],
    truncated: false,
  ),
);

ReviewCardModel _card({Iterable<ReviewCitationModel> citations = const []}) {
  const front = 'Why does the source matter?';
  const answer = 'It lets the learner inspect the evidence.';
  return ReviewCardModel(
    front: front,
    frontRuns: [_run(front)],
    back: const [answer],
    backRuns: [
      [_run(answer)],
    ],
    backUnits: [
      ReviewSentenceModel(text: answer, runs: [_run(answer)]),
    ],
    answerSteps: const [ReviewAnswerLineModel(backFrom: 0, backTo: 1)],
    reshaped: false,
    note: const [],
    images: const [],
    imagesBack: const [],
    citations: citations,
  );
}

ReviewStateModel _state(ReviewCardModel card) => ReviewStateModel(
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

class _Harness extends StatelessWidget {
  const _Harness({
    required this.card,
    required this.attempt,
    this.revealed = true,
  });

  final ReviewCardModel card;
  final TextEditingController attempt;
  final bool revealed;

  @override
  Widget build(BuildContext context) {
    return RepaintBoundary(
      key: const ValueKey('shot'),
      child: MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: alixDark(),
        home: Scaffold(
          body: ReviewCardView(
            state: _state(card),
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
            onOpenTopic: (_) {},
          ),
        ),
      ),
    );
  }
}

Future<void> _pump(
  WidgetTester tester,
  ReviewCardModel card,
  TextEditingController attempt, {
  bool revealed = true,
}) async {
  tester.view.physicalSize = const Size(1080, 2280);
  tester.view.devicePixelRatio = 3.0;
  await tester.pumpWidget(
    _Harness(card: card, attempt: attempt, revealed: revealed),
  );
  await tester.pumpAndSettle();
}

Future<void> _open(WidgetTester tester) async {
  await tester.tap(find.byKey(const ValueKey('answer-tap')));
  await tester.pumpAndSettle();
}

Future<void> _source(WidgetTester tester) async {
  await tester.tap(find.byKey(const ValueKey('answer-sheet-source')));
  await tester.pumpAndSettle();
}

Future<void> _loadFonts() async {
  final sans = FontLoader('IBM Plex Sans');
  for (final weight in const ['400', '500', '600', '700']) {
    sans.addFont(_bytes('assets/fonts/ibm-plex-sans-$weight.ttf'));
  }
  final mono = FontLoader('IBM Plex Mono');
  for (final weight in const ['400', '600']) {
    mono.addFont(_bytes('assets/fonts/ibm-plex-mono-$weight.ttf'));
  }
  await sans.load();
  await mono.load();
}

Future<ByteData> _bytes(String path) async {
  final bytes = await File(path).readAsBytes();
  return bytes.buffer.asByteData();
}

Future<void> _render(WidgetTester tester, String name) async {
  final dir = Platform.environment['ALIX_RENDER_DIR'];
  if (dir == null) return;
  final boundary = tester.renderObject<RenderRepaintBoundary>(
    find.byKey(const ValueKey('shot')),
  );
  final image = await boundary.toImage(pixelRatio: 3);
  final png = await image.toByteData(format: ui.ImageByteFormat.png);
  await File(
    '$dir/phone-source-$name.png',
  ).writeAsBytes(png!.buffer.asUint8List());
}

void main() {
  testWidgets('only a cited card offers the Hide and Source switch', (
    tester,
  ) async {
    addTearDown(tester.view.reset);
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);

    await _pump(tester, _card(), attempt);
    await _open(tester);
    expect(find.byKey(const ValueKey('answer-sheet-source')), findsNothing);
    await tester.tap(find.byKey(const ValueKey('hide-sheet-close')));
    await tester.pumpAndSettle();

    await _pump(
      tester,
      _card(citations: [_citation('return evidence;')]),
      attempt,
    );
    await _open(tester);
    expect(find.byKey(const ValueKey('answer-sheet-hide')), findsOneWidget);
    expect(find.byKey(const ValueKey('answer-sheet-source')), findsOneWidget);
  });

  testWidgets('a card remembers its last shown view until the next card', (
    tester,
  ) async {
    addTearDown(tester.view.reset);
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    final first = _card(citations: [_citation('return first;')]);
    final reread = _card(citations: [_citation('return first;')]);
    final second = _card(citations: [_citation('return second;')]);

    await _pump(tester, first, attempt);
    await _open(tester);
    await _source(tester);
    expect(
      tester.getRect(find.byKey(const ValueKey('hide-sheet'))).top,
      56,
      reason: 'Source reaches the bottom of the session header',
    );
    await tester.tap(find.byKey(const ValueKey('hide-sheet-close')));
    await tester.pumpAndSettle();

    await _pump(tester, reread, attempt);
    await _open(tester);
    expect(
      find.byKey(const ValueKey('source-sheet')),
      findsOneWidget,
      reason: 'a state re-read of the same card, as a tutor note causes, '
          'keeps Source',
    );
    expect(find.text('return first;'), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('hide-sheet-close')));
    await tester.pumpAndSettle();

    await _pump(tester, second, attempt, revealed: false);
    await _pump(tester, second, attempt);
    await _open(tester);
    expect(
      find.byKey(const ValueKey('hide-sheet-hide-view')),
      findsOneWidget,
      reason: 'the next card, entered unrevealed, opens on Hide',
    );
  });

  testWidgets('Source shows an honest citation error line', (tester) async {
    addTearDown(tester.view.reset);
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    final card = _card(
      citations: const [
        ReviewCitationModel(
          locator: 'src:missing.rs:4',
          error: 'source excerpt changed or disappeared',
        ),
      ],
    );

    await _pump(tester, card, attempt);
    await _open(tester);
    await _source(tester);
    expect(find.byKey(const ValueKey('source-error')), findsOneWidget);
    expect(find.text('source excerpt changed or disappeared'), findsOneWidget);
  });

  testWidgets('Source keeps code upright and scrolls a long line sideways, never wrapping', (
    tester,
  ) async {
    addTearDown(tester.view.reset);
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    const long =
        'final resolvedEvidence = repository.readEveryRelevantSourceLineWithoutTruncatingTheResult();';
    await _pump(
      tester,
      _card(citations: [_citation(long), _citation('return ok;')]),
      attempt,
    );
    await _open(tester);
    await _source(tester);

    expect(find.byType(RotatedBox), findsNothing, reason: 'step 1: nothing turns sideways');
    final longLine = find.text(long);
    final shortLine = find.text('return ok;');
    expect(
      tester.getSize(longLine).height,
      tester.getSize(shortLine).height,
      reason: 'step 2: the long line keeps one line height (no wrap)',
    );
    final before = tester.getTopLeft(longLine).dx;
    await tester.dragFrom(
      tester.getTopLeft(longLine) + const Offset(20, 5),
      const Offset(-300, 0),
    );
    await tester.pumpAndSettle();
    expect(
      tester.getTopLeft(longLine).dx < before,
      isTrue,
      reason: 'step 3: a sideways drag scrolls the code',
    );
    expect(tester.takeException(), isNull, reason: 'step 4: no layout overflow');
  });

  for (final (name, citation, showSource) in [
    ('hide-cited', _citation('return evidence;'), false),
    ('source-fitting', _citation('return evidence;'), true),
    (
      'source-rotated',
      _citation(
        'final resolvedEvidence = repository.readEveryRelevantSourceLineWithoutTruncatingTheResult();',
      ),
      true,
    ),
    (
      'source-error',
      const ReviewCitationModel(
        locator: 'src:missing.rs:4',
        error: 'source excerpt changed or disappeared',
      ),
      true,
    ),
  ]) {
    testWidgets('real-font render: $name', (tester) async {
      addTearDown(tester.view.reset);
      final attempt = TextEditingController();
      addTearDown(attempt.dispose);
      await tester.runAsync(() async {
        await _loadFonts();
        await _pump(tester, _card(citations: [citation]), attempt);
        await _open(tester);
        if (showSource) await _source(tester);
        await _render(tester, name);
      });
    });
  }
}
