import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bridge/review_bridge.dart';
import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/shared/blank_pill.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

import 'support/deck_fixture.dart';

ReviewCardModel groupCard() => openCard(
  '## carpals\n'
  'The scaphoid and the lunate.\n'
  '<!-- blank: span [w] hidden="scaphoid" b:a1b2c3 -->\n'
  '<!-- blank: span [w] hidden="lunate" b:d4e5f6 -->\n'
  '<!-- id: card-group -->\n',
);

ReviewCardModel openCard(String body) {
  final root = Directory.systemTemp.createTempSync('alix-cloze-reveal-');
  addTearDown(() => root.deleteSync(recursive: true));
  final deck = '${root.path}/carpals.md';
  writeTestDeck(deck, body);
  return const ReviewBridgeFactory()
      .open(deckPath: deck, rootDir: root.path)
      .state
      .card!;
}

class RevealRow {
  const RevealRow(
    this.label,
    this.mode, {
    this.introducing = false,
    this.revealed = false,
    this.revealedLines = 0,
    this.choice,
    this.check,
    required this.filled,
  });

  final String label;
  final ReviewMode mode;
  final bool introducing;
  final bool revealed;
  final int revealedLines;
  final ReviewChoiceFeedbackModel? choice;
  final ReviewCheckFeedbackModel? check;
  final int filled;
}

Future<void> pumpRow(
  WidgetTester tester,
  ReviewCardModel card,
  RevealRow row, {
  ThemeData? theme,
}) async {
  final attempt = TextEditingController();
  addTearDown(attempt.dispose);
  final typed = [for (final _ in card.gradeableSteps) TextEditingController()];
  for (final controller in typed) {
    addTearDown(controller.dispose);
  }
  final choiceMode = row.mode == ReviewMode.choice;
  await tester.pumpWidget(
    MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: theme ?? alixDark(),
      home: Scaffold(
        body: ReviewCardView(
          state: ReviewStateModel(
            card: card,
            mode: row.mode,
            depth: ReviewDepth.recall,
            introducing: row.introducing,
            choices: choiceMode ? const ['option one', 'option two'] : null,
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
          revealed: row.revealed,
          revealedLines: row.revealedLines,
          choice: row.choice,
          multiChoice: null,
          multiSelected: const {},
          checkFeedback: row.check,
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
          typedControllers: typed,
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

Iterable<TextSpan> spansOf(WidgetTester tester) sync* {
  Iterable<TextSpan> walk(InlineSpan span) sync* {
    if (span is TextSpan) {
      yield span;
      for (final child in span.children ?? const <InlineSpan>[]) {
        yield* walk(child);
      }
    }
  }

  for (final rich in tester.widgetList<RichText>(find.byType(RichText))) {
    yield* walk(rich.text);
  }
}

void main() {
  setUpAll(() async => RustLib.init());

  testWidgets('every mode fills the question in place and never repeats it', (
    tester,
  ) async {
    final card = groupCard();
    final typedPass = ReviewCheckFeedbackModel(
      results: const [
        ReviewTypedResultModel(input: 'typed one', expected: '', passed: true),
        ReviewTypedResultModel(input: 'typed two', expected: '', passed: true),
      ],
      passed: true,
    );
    const chosen = ReviewChoiceFeedbackModel(
      chosen: 0,
      correct: 0,
      passed: true,
    );
    final rows = [
      const RevealRow('flip before reveal', ReviewMode.flip, filled: 0),
      const RevealRow(
        'flip after reveal',
        ReviewMode.flip,
        revealed: true,
        filled: 2,
      ),
      const RevealRow('line, no press', ReviewMode.lineByLine, filled: 0),
      const RevealRow(
        'line, one press',
        ReviewMode.lineByLine,
        revealedLines: 1,
        filled: 1,
      ),
      const RevealRow(
        'line, two presses',
        ReviewMode.lineByLine,
        revealedLines: 2,
        filled: 2,
      ),
      const RevealRow('typing before check', ReviewMode.typing, filled: 0),
      RevealRow(
        'typing at feedback',
        ReviewMode.typing,
        check: typedPass,
        filled: 2,
      ),
      const RevealRow('choice before choosing', ReviewMode.choice, filled: 0),
      const RevealRow(
        'choice at feedback',
        ReviewMode.choice,
        choice: chosen,
        filled: 2,
      ),
      const RevealRow(
        'intro before reveal',
        ReviewMode.flip,
        introducing: true,
        filled: 0,
      ),
      const RevealRow(
        'intro after reveal',
        ReviewMode.lineByLine,
        introducing: true,
        revealed: true,
        filled: 2,
      ),
    ];
    final accent = alixDark().alix.boltHi;
    for (final row in rows) {
      await pumpRow(tester, card, row);
      final step = '${row.label} (filled ${row.filled})';
      expect(
        find.byType(BlankPill),
        findsNWidgets(2 - row.filled),
        reason: '$step: unfilled blanks stay pills',
      );
      final words = ['scaphoid', 'lunate'];
      for (final (index, word) in words.indexed) {
        final shown = spansOf(
          tester,
        ).where((span) => (span.text ?? '').contains(word)).toList();
        final expected = index < row.filled ? 1 : 0;
        expect(
          shown.length,
          expected,
          reason: '$step: "$word" drawn $expected time(s), got $shown',
        );
        for (final span in shown) {
          expect(span.style?.color, accent, reason: '$step: "$word" accent');
        }
      }
    }
  });

  testWidgets('a blank inside a code fence is a pill until it is filled', (
    tester,
  ) async {
    final card = openCard(
      '## the borrow\n'
      '```rust\n'
      'let x = &mut y;\n'
      '```\n'
      '<!-- blank: span hidden="mut" b:a1b2c3 -->\n'
      '<!-- id: card-fence -->\n',
    );
    String code() => spansOf(tester)
        .map((span) => span.text ?? '')
        .where((text) => text.contains('let x'))
        .join('|');

    await pumpRow(
      tester,
      card,
      const RevealRow('masked', ReviewMode.flip, filled: 0),
    );
    expect(find.byType(BlankPill), findsOneWidget, reason: 'masked: one pill');
    expect(code(), isNot(contains(RegExp('⍰|⬚|mut'))), reason: code());

    await pumpRow(
      tester,
      card,
      const RevealRow('revealed', ReviewMode.flip, revealed: true, filled: 1),
    );
    expect(find.byType(BlankPill), findsNothing, reason: 'revealed: no pill');
    expect(code(), contains('&mut y'), reason: 'revealed: ${code()}');
  });
}
