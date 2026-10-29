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
import 'package:alix_mobile/shared/inline_runs.dart';
import 'package:alix_mobile/theme.dart';

const _note =
    'A note stays at the left edge of its region, whatever its width.';

InlineRunModel _run(String text, {bool code = false}) =>
    InlineRunModel(text: text, bold: false, italic: false, code: code);

ReviewSentenceModel _sentence(String text, {bool code = false}) =>
    ReviewSentenceModel(
      text: text,
      runs: [_run(text, code: code)],
    );

Widget _card({
  required String front,
  required String answer,
  required bool codeAnswer,
  required TextEditingController attempt,
}) {
  final card = ReviewCardModel(
    front: front,
    frontRuns: [_run(front)],
    back: [answer],
    backRuns: [
      [_run(answer, code: codeAnswer)],
    ],
    backUnits: [_sentence(answer, code: codeAnswer)],
    answerSteps: const [ReviewAnswerLineModel(backFrom: 0, backTo: 1)],
    reshaped: false,
    note: [
      ReviewNoteModel(badge: ReviewBadge.note, units: [_sentence(_note)]),
    ],
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
  return RepaintBoundary(
    key: const ValueKey('shot'),
    child: MaterialApp(
      debugShowCheckedModeBanner: false,
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
          onOpenSection: (_) {},
        ),
      ),
    ),
  );
}

Finder _runs(String text) => find.byWidgetPredicate(
  (widget) => widget is InlineRuns && widget.runs.first.text == text,
);

/// The content column a block is laid out in: the region's padding box
/// deflated by its own padding.
Rect _column(WidgetTester tester, Finder padding) {
  final insets = tester
      .widget<Padding>(padding)
      .padding
      .resolve(TextDirection.ltr);
  return insets.deflateRect(tester.getRect(padding));
}

Finder _regionPadding(Finder inside) =>
    find.ancestor(of: inside, matching: find.byType(Padding)).first;

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
  await File('$dir/phone-$name.png').writeAsBytes(png!.buffer.asUint8List());
}

void main() {
  const long =
      'Which of the statements an expression statement may hold is complete by '
      'itself, without any trailing expression, in the grammar of the language '
      'as the reference describes it?';
  const longAnswer =
      'The empty statement, a lone semicolon, is complete by itself; everything '
      'else in the statement grammar needs an expression, a declaration or a '
      'block to carry it.';
  for (final (name, front, answer, code, wide) in [
    ('narrow', 'Why?', ';', true, false),
    ('wide', long, longAnswer, false, true),
  ]) {
    testWidgets(
      'a $name question and answer are start-aligned blocks centred in the '
      'column, and the note keeps its full width at the left',
      (tester) async {
        addTearDown(tester.view.reset);
        tester.view.physicalSize = const Size(1080, 2280);
        tester.view.devicePixelRatio = 3.0;
        final attempt = TextEditingController();
        addTearDown(attempt.dispose);

        await tester.runAsync(() async {
          await _loadFonts();
          await tester.pumpWidget(
            _card(
              front: front,
              answer: answer,
              codeAnswer: code,
              attempt: attempt,
            ),
          );
          await tester.pumpAndSettle();
          await _render(tester, name);
        });

        for (final (side, text) in [('question', front), ('answer', answer)]) {
          final block = _runs(text);
          expect(block, findsOneWidget, reason: 'the $side prose renders');
          expect(
            tester.widget<InlineRuns>(block).textAlign,
            TextAlign.start,
            reason: 'the $side text is start-aligned',
          );
          final rect = tester.getRect(block);
          final column = _column(tester, _regionPadding(block));
          expect(
            (rect.center.dx - column.center.dx).abs(),
            lessThanOrEqualTo(1),
            reason:
                'the $side block centre ${rect.center.dx} sits on the column '
                'centre ${column.center.dx}',
          );
          if (wide) {
            expect(
              (rect.width - column.width).abs(),
              lessThanOrEqualTo(1),
              reason:
                  'the $side block ${rect.width} fills its column '
                  '${column.width}',
            );
          } else {
            expect(
              rect.width,
              lessThan(column.width / 2),
              reason:
                  'the $side block ${rect.width} is narrower than half the '
                  'column ${column.width}',
            );
          }
        }

        final note = _runs(_note);
        final noteRect = tester.getRect(note);
        final noteBlock = tester.getRect(
          find.ancestor(of: note, matching: find.byType(NoteBlock)).first,
        );
        final noteColumn = _column(
          tester,
          find.byKey(const ValueKey('note-content')),
        );
        expect(
          (noteBlock.width - noteColumn.width).abs(),
          lessThanOrEqualTo(1),
          reason:
              'the note block ${noteBlock.width} spans its column '
              '${noteColumn.width}',
        );
        final noteLines = tester.getRect(
          find.ancestor(of: note, matching: find.byType(Column)).first,
        );
        expect(
          (noteRect.left - noteLines.left).abs(),
          lessThanOrEqualTo(1),
          reason:
              'the note text ${noteRect.left} starts at the left edge of its '
              'block ${noteLines.left}',
        );
      },
    );
  }
}
