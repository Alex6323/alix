import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/shared/inline_runs.dart';
import 'package:alix_mobile/theme.dart';

ReviewNoteModel _note(String text, {ReviewBadge badge = ReviewBadge.note}) =>
    ReviewNoteModel(
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

Widget _card(
  List<ReviewNoteModel> notes,
  TextEditingController attempt, {
  List<ReviewContentUnitModel>? frontUnits,
  ThemeData? theme,
}) {
  final card = ReviewCardModel(
    front: 'Question',
    frontRuns: const [],
    frontUnits: frontUnits,
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
    theme: theme ?? alixDark(),
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
  );
}

// A note's own block is the innermost Container around its body: the block
// carries no width of its own any more, so there is nothing else to key on.
Finder _noteBox(String body) =>
    find.ancestor(of: find.text(body), matching: find.byType(Container)).first;

/// The bar is the block's left border, the only decoration a note carries.
BorderSide _bar(WidgetTester tester, String body) {
  final box = tester.widget<Container>(_noteBox(body));
  return ((box.decoration! as BoxDecoration).border! as Border).left;
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
    expect(_noteBox('First.'), findsOneWidget);
    expect(_noteBox('Second.'), findsOneWidget);
    expect(
      tester.getRect(_noteBox('First.')),
      isNot(tester.getRect(_noteBox('Second.'))),
      reason: 'two notes stack in their own boxes instead of merging',
    );
  });

  testWidgets('a quotation inside a note uses the note prose alignment', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    const quoted = 'A quoted passage.';
    final note = ReviewNoteModel(
      badge: ReviewBadge.note,
      units: [
        ReviewQuoteModel([
          ReviewSentenceModel(
            text: quoted,
            runs: [
              InlineRunModel(
                text: quoted,
                bold: false,
                italic: false,
                code: false,
              ),
            ],
          ),
        ]),
      ],
    );

    await tester.pumpWidget(_card([note], attempt));

    final runs = tester.widget<InlineRuns>(
      find
          .ancestor(of: find.text(quoted), matching: find.byType(InlineRuns))
          .first,
    );
    expect(runs.textAlign, TextAlign.start);
  });

  testWidgets('a checklist inside a note uses the note prose alignment', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    const itemText = 'A checklist item that belongs to the authored note.';
    final note = ReviewNoteModel(
      badge: ReviewBadge.note,
      units: [
        ReviewChecklistModel([
          ReviewChecklistItemModel(
            checked: true,
            text: itemText,
            runs: const [
              InlineRunModel(
                text: itemText,
                bold: false,
                italic: false,
                code: false,
              ),
            ],
          ),
        ]),
      ],
    );

    await tester.pumpWidget(_card([note], attempt));

    final runs = tester.widget<InlineRuns>(
      find
          .ancestor(of: find.text(itemText), matching: find.byType(InlineRuns))
          .first,
    );
    expect(runs.textAlign, TextAlign.start);
  });

  testWidgets('a checklist on the centred question keeps its rows at the start', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    const itemText = 'A checklist item that belongs to the question.';

    await tester.pumpWidget(
      _card(
        const [],
        attempt,
        frontUnits: [
          ReviewChecklistModel([
            ReviewChecklistItemModel(
              checked: false,
              text: itemText,
              runs: const [
                InlineRunModel(
                  text: itemText,
                  bold: false,
                  italic: false,
                  code: false,
                ),
              ],
            ),
          ]),
        ],
      ),
    );

    final runs = tester.widget<InlineRuns>(
      find
          .ancestor(of: find.text(itemText), matching: find.byType(InlineRuns))
          .first,
    );
    expect(runs.textAlign, TextAlign.start);
  });

  testWidgets('a quotation on the question sets its prose at the start', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    const quoted = 'A quoted passage that belongs to the question.';

    await tester.pumpWidget(
      _card(
        const [],
        attempt,
        frontUnits: [
          ReviewQuoteModel([
            ReviewSentenceModel(
              text: quoted,
              runs: [
                InlineRunModel(
                  text: quoted,
                  bold: false,
                  italic: false,
                  code: false,
                ),
              ],
            ),
          ]),
        ],
      ),
    );

    final runs = tester.widget<InlineRuns>(
      find
          .ancestor(of: find.text(quoted), matching: find.byType(InlineRuns))
          .first,
    );
    expect(runs.textAlign, TextAlign.start);
  });

  testWidgets('a checklist nested in a question quotation stays left aligned', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    const itemText =
        'A checklist item inside the quoted question that wraps across more '
        'than one line on a phone-sized review card.';

    await tester.pumpWidget(
      _card(
        const [],
        attempt,
        frontUnits: [
          ReviewQuoteModel([
            ReviewChecklistModel([
              ReviewChecklistItemModel(
                checked: false,
                text: itemText,
                runs: const [
                  InlineRunModel(
                    text: itemText,
                    bold: false,
                    italic: false,
                    code: false,
                  ),
                ],
              ),
            ]),
          ]),
        ],
      ),
    );

    final runs = tester.widget<InlineRuns>(
      find
          .ancestor(of: find.text(itemText), matching: find.byType(InlineRuns))
          .first,
    );
    expect(runs.textAlign, TextAlign.start);
  });

  testWidgets('a sentence in nested question quotations stays at the start', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    const quoted = 'A sentence inside two quotations on the question.';

    await tester.pumpWidget(
      _card(
        const [],
        attempt,
        frontUnits: [
          ReviewQuoteModel([
            ReviewQuoteModel([
              ReviewSentenceModel(
                text: quoted,
                runs: const [
                  InlineRunModel(
                    text: quoted,
                    bold: false,
                    italic: false,
                    code: false,
                  ),
                ],
              ),
            ]),
          ]),
        ],
      ),
    );

    final runs = tester.widget<InlineRuns>(
      find
          .ancestor(of: find.text(quoted), matching: find.byType(InlineRuns))
          .first,
    );
    expect(runs.textAlign, TextAlign.start);
  });

  testWidgets('a checklist in nested question quotations stays left aligned', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    const itemText = 'A checklist item inside two quotations on the question.';

    await tester.pumpWidget(
      _card(
        const [],
        attempt,
        frontUnits: [
          ReviewQuoteModel([
            ReviewQuoteModel([
              ReviewChecklistModel([
                ReviewChecklistItemModel(
                  checked: false,
                  text: itemText,
                  runs: const [
                    InlineRunModel(
                      text: itemText,
                      bold: false,
                      italic: false,
                      code: false,
                    ),
                  ],
                ),
              ]),
            ]),
          ]),
        ],
      ),
    );

    final runs = tester.widget<InlineRuns>(
      find
          .ancestor(of: find.text(itemText), matching: find.byType(InlineRuns))
          .first,
    );
    expect(runs.textAlign, TextAlign.start);
  });

  testWidgets('note prose is ragged right at every width and text scale', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    addTearDown(() => tester.binding.setSurfaceSize(null));
    addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
    const body = 'A note long enough to be prose.';

    const rows = [
      (width: 800.0, scale: 1.0),
      (width: 415.0, scale: 1.0),
      (width: 360.0, scale: 1.0),
      (width: 800.0, scale: 1.5),
    ];
    for (final row in rows) {
      await tester.binding.setSurfaceSize(Size(row.width, 800));
      tester.platformDispatcher.textScaleFactorTestValue = row.scale;
      await tester.pumpWidget(
        _card([_note(body, badge: ReviewBadge.note)], attempt),
      );

      final runs = tester.widget<InlineRuns>(
        find
            .ancestor(of: find.text(body), matching: find.byType(InlineRuns))
            .first,
      );
      expect(
        runs.textAlign,
        TextAlign.start,
        reason: 'surface ${row.width} at text scale ${row.scale}',
      );
    }
  });

  testWidgets('every badge paints its GitHub colour on the bar and the word', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    const sets = {
      Brightness.dark: {
        ReviewBadge.note: Color(0xFF4493F8),
        ReviewBadge.tip: Color(0xFF3FB950),
        ReviewBadge.important: Color(0xFFAB7DF8),
        ReviewBadge.warning: Color(0xFFD29922),
        ReviewBadge.caution: Color(0xFFF85149),
      },
      Brightness.light: {
        ReviewBadge.note: Color(0xFF0969DA),
        ReviewBadge.tip: Color(0xFF1A7F37),
        ReviewBadge.important: Color(0xFF8250DF),
        ReviewBadge.warning: Color(0xFF9A6700),
        ReviewBadge.caution: Color(0xFFCF222E),
      },
    };

    for (final MapEntry(key: brightness, value: colours) in sets.entries) {
      final theme = brightness == Brightness.dark ? alixDark() : alixLight();
      for (final badge in ReviewBadge.values) {
        await tester.pumpWidget(
          _card([_note('Body.', badge: badge)], attempt, theme: theme),
        );
        // MaterialApp animates a theme change over several frames.
        await tester.pumpAndSettle();
        final name = badge.name.toUpperCase();
        expect(find.text(name), findsOneWidget, reason: '$badge names itself');
        final bar = _bar(tester, 'Body.');
        expect(bar.width, 4, reason: '$brightness $badge: the bar is 4 wide');
        expect(
          bar.color,
          colours[badge],
          reason: '$brightness $badge: the bar takes the GitHub colour',
        );
        expect(
          tester.widget<Text>(find.text(name)).style!.color,
          colours[badge],
          reason: '$brightness $badge: the word takes the same colour',
        );
        expect(
          find.text(name),
          findsOneWidget,
          reason: '$badge is named once, by the word above the text',
        );
      }
    }
  });

  testWidgets('a note carries no box and no hairline above it', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);

    await tester.pumpWidget(_card([_note('A table column.')], attempt));

    final box = tester.widget<Container>(_noteBox('A table column.'));
    final decoration = box.decoration! as BoxDecoration;
    expect(decoration.color, isNull, reason: 'no wash');
    final border = decoration.border! as Border;
    expect(border.top, BorderSide.none);
    expect(border.right, BorderSide.none);
    expect(border.bottom, BorderSide.none);
    expect(
      find.descendant(
        of: find.byType(ReviewCardView),
        matching: find.byWidgetPredicate(
          (widget) =>
              widget is Container &&
              widget.constraints?.maxHeight == 1 &&
              widget.decoration != null,
        ),
      ),
      findsOneWidget,
      reason: 'one hairline remains, between the question and the answer',
    );
  });
}
