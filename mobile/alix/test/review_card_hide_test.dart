// Hiding a revealed answer for a self-test on the phone: a tap on the answer
// region veils every word in place (first and last letters kept, the rest
// transparent, inline code too), the note is invisible with its size kept,
// nothing moves, and a second tap shows everything again. A card with
// nothing to veil has no tap target.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/theme.dart';

const _front = 'What does `let` without `mut` give you in Rust?';
const _line1 = 'An immutable binding: the name cannot be reassigned.';
const _line2 = 'Shadow it with let instead.';
const _note = 'Immutability is the default so a reader knows a name keeps its value.';

InlineRunModel _run(String text, {bool code = false}) =>
    InlineRunModel(text: text, bold: false, italic: false, code: code);

ReviewCardModel _card() => ReviewCardModel(
  front: _front,
  frontRuns: [_run(_front)],
  back: const [_line1, _line2],
  backRuns: [
    [_run(_line1)],
    [_run('Shadow it with '), _run('let', code: true), _run(' instead.')],
  ],
  backUnits: [
    ReviewSentenceModel(text: _line1, runs: [_run(_line1)]),
    ReviewSentenceModel(
      text: _line2,
      runs: [_run('Shadow it with '), _run('let', code: true), _run(' instead.')],
    ),
  ],
  answerSteps: const [
    ReviewAnswerLineModel(backFrom: 0, backTo: 1),
    ReviewAnswerLineModel(backFrom: 1, backTo: 2),
  ],
  reshaped: false,
  note: [
    ReviewNoteModel(
      badge: ReviewBadge.note,
      units: [ReviewSentenceModel(text: _note, runs: [_run(_note)])],
    ),
  ],
  images: const [],
  imagesBack: const [],
);

ReviewStateModel _state({List<String>? choices}) => ReviewStateModel(
  card: _card(),
  mode: choices == null ? ReviewMode.flip : ReviewMode.choice,
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
  choices: choices,
  choiceRuns: choices?.map((text) => [_run(text)]).toList(),
);

class _Harness extends StatefulWidget {
  const _Harness({required this.state, required this.revealed, required this.attempt});

  final ReviewStateModel state;
  final bool revealed;
  final TextEditingController attempt;

  @override
  State<_Harness> createState() => _HarnessState();
}

class _HarnessState extends State<_Harness> {
  bool hidden = false;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      theme: alixDark(),
      home: Scaffold(
        body: ReviewCardView(
          state: widget.state,
          revealed: widget.revealed,
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
          attemptController: widget.attempt,
          typedControllers: const [],
          serverLive: false,
          tutorCard: null,
          verdictGrade: ReviewGrade.pass,
          answerHidden: hidden,
          onToggleHidden: () => setState(() => hidden = !hidden),
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
    );
  }
}

final _answerTap = find.byKey(const ValueKey('answer-tap'));
final _questionRegion = find.byKey(const ValueKey('question-region'));
final _answerRegion = find.byKey(const ValueKey('answer-region'));
final _noteRegion = find.byKey(const ValueKey('note-region'));
final _noteContent = find.byKey(const ValueKey('note-content'));

/// The painted runs of a rich text, flattened: (text, colour) per span.
List<(String, Color?)> _spans(WidgetTester tester, String plain) {
  final text = tester.widget<Text>(find.text(plain));
  final out = <(String, Color?)>[];
  text.textSpan!.visitChildren((span) {
    if (span is TextSpan && span.text != null && span.text!.isNotEmpty) {
      out.add((span.text!, span.style?.color));
    }
    return true;
  });
  return out;
}

Future<void> _pump(WidgetTester tester, {bool revealed = true, List<String>? choices}) async {
  addTearDown(tester.view.reset);
  tester.view.physicalSize = const Size(1080, 2280);
  tester.view.devicePixelRatio = 3.0;
  final attempt = TextEditingController();
  addTearDown(attempt.dispose);
  await tester.pumpWidget(
    _Harness(state: _state(choices: choices), revealed: revealed, attempt: attempt),
  );
  await tester.pumpAndSettle();
}

void main() {
  testWidgets(
    'law: a tap on the answer veils every word in place, hides the note with its box kept, and a second tap shows it all again',
    (tester) async {
      await _pump(tester);
      expect(_answerTap, findsOneWidget, reason: 'a revealed answer is the tap target');
      final before = {
        for (final (name, finder) in [
          ('question', _questionRegion),
          ('answer', _answerRegion),
          ('note', _noteRegion),
          ('note content', _noteContent),
          ('line 1', find.text(_line1)),
          ('line 2', find.text(_line2)),
        ])
          name: tester.getRect(finder),
      };
      expect(
        _spans(tester, _line1).every((span) => span.$2 != Colors.transparent),
        isTrue,
        reason: 'shown: no run is transparent',
      );

      await tester.tap(_answerTap);
      await tester.pumpAndSettle();

      final spans = _spans(tester, _line1);
      expect(
        spans.take(4).map((span) => span.$1).toList(),
        ['A', 'n', ' i', 'mmutabl'],
        reason: 'An immutable: the runs split at the first and last letters',
      );
      expect(
        spans.take(4).map((span) => span.$2 == Colors.transparent).toList(),
        [false, true, false, true],
        reason: 'An immutable: first and last letters kept, the middle veiled',
      );
      expect(
        spans.map((span) => span.$1).join(),
        _line1,
        reason: 'hidden: the whole line is still drawn, in place',
      );
      final code = _spans(tester, _line2);
      expect(
        code.map((span) => span.$1).join(),
        _line2,
        reason: 'hidden: the line with the code run is whole',
      );
      final at = code.indexWhere((span) => span.$1 == 'l');
      final codeRuns = code.sublist(at, at + 3);
      expect(codeRuns.map((span) => span.$1).toList(), ['l', 'e', 't']);
      expect(
        codeRuns.map((span) => span.$2 == Colors.transparent).toList(),
        [false, true, false],
        reason: 'the code run let is veiled by the same rule: $codeRuns',
      );
      final noteText = find.text(_note);
      expect(noteText, findsOneWidget, reason: 'hidden: the note is still laid out');
      expect(
        tester
            .widget<Visibility>(
              find.ancestor(of: noteText, matching: find.byType(Visibility)).first,
            )
            .visible,
        isFalse,
        reason: 'hidden: the note is invisible',
      );
      for (final (name, finder) in [
        ('question', _questionRegion),
        ('answer', _answerRegion),
        ('note', _noteRegion),
        ('note content', _noteContent),
        ('line 1', find.text(_line1)),
        ('line 2', find.text(_line2)),
      ]) {
        expect(tester.getRect(finder), before[name], reason: 'hidden: $name did not move');
      }

      await tester.tap(_answerTap);
      await tester.pumpAndSettle();
      expect(
        _spans(tester, _line1).every((span) => span.$2 != Colors.transparent),
        isTrue,
        reason: 'shown again: every run reads',
      );
      expect(
        tester
            .widget<Visibility>(
              find.ancestor(of: find.text(_note), matching: find.byType(Visibility)).first,
            )
            .visible,
        isTrue,
        reason: 'shown again: the note is back',
      );
      for (final (name, finder) in [('answer', _answerRegion), ('note', _noteRegion)]) {
        expect(tester.getRect(finder), before[name], reason: 'shown again: $name did not move');
      }
    },
  );

  testWidgets('an unrevealed card and a choice card have no tap target', (tester) async {
    await _pump(tester, revealed: false);
    expect(_answerTap, findsNothing, reason: 'nothing to hide before the reveal');
    await _pump(tester, revealed: false, choices: ['a', 'b', 'c']);
    expect(_answerTap, findsNothing, reason: 'options are picked, not hidden');
  });
}
