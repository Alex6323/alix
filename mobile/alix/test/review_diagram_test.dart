import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_svg/flutter_svg.dart';

import 'package:alix_mobile/review/masked_image.dart';
import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/theme.dart';

void main() {
  testWidgets(
    'ReviewCardView renders supplied front math units and keeps a plain '
    'front line literal',
    (tester) async {
      final attempt = TextEditingController();
      addTearDown(attempt.dispose);
      const svg =
          '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="12" '
          'viewBox="0 0 40 12"><path d="M0 0h40v12H0z"/></svg>';
      const mathRun = InlineRunModel(
        text: 'x^2',
        bold: false,
        italic: false,
        code: false,
        math: InlineMathModel(display: true, svg: svg),
      );
      ReviewSentenceModel sentence(String line) => ReviewSentenceModel(
        text: line,
        runs: [
          InlineRunModel(text: line, bold: false, italic: false, code: false),
        ],
      );
      ReviewCardModel card(List<ReviewContentUnitModel> units) {
        return ReviewCardModel(
          front: 'Topic',
          frontRuns: const [],
          frontUnits: [sentence('Topic'), ...units],
          context: const [],
          contextRuns: const [],
          back: const ['answer'],
          backRuns: const [[]],
          backUnits: const [],
          answerSteps: const [ReviewAnswerLineModel(backFrom: 0, backTo: 1)],
          reshaped: false,
          note: const [],
          images: const [],
          imagesBack: const [],
        );
      }

      Widget app(ReviewCardModel card) {
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
          dueLeft: 0,
          newLeft: 0,
        );
        return MaterialApp(
          theme: alixDark(),
          home: Scaffold(
            body: ReviewCardView(
              state: state,
              revealed: false,
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
              serverLive: true,
              tutorCard: null,
              verdictGrade: ReviewGrade.fail,
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

      final unit = ReviewSentenceModel(text: 'x^2', runs: [mathRun]);
      await tester.pumpWidget(app(card([unit])));
      expect(find.bySemanticsLabel('x^2'), findsOneWidget);
      expect(find.byType(SvgPicture), findsOneWidget);
      expect(
        find.textContaining('math could not render', findRichText: true),
        findsNothing,
      );

      await tester.pumpWidget(app(card([sentence('\$\$'), sentence('x^2')])));
      expect(find.text('\$\$'), findsOneWidget);
      expect(find.text('x^2'), findsOneWidget);

      await tester.pumpWidget(
        app(
          card([
            ReviewCodeModel(const ['before', '```', 'after']),
            unit,
          ]),
        ),
      );
      expect(find.bySemanticsLabel('x^2'), findsOneWidget);
      expect(find.textContaining('before'), findsOneWidget);
    },
  );

  testWidgets('complete line reveal renders the resolved diagram unit', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    final source = File('assets/icon/alix-192.png').absolute.path;
    for (final back in const [
      ['```mermaid', 'flowchart LR', ' A-->B', '```'],
      ['````mermaid', 'flowchart LR', ' A-->B', '````'],
    ]) {
      final card = ReviewCardModel(
        front: 'Question',
        frontRuns: const [],
        context: const [],
        contextRuns: const [],
        back: back,
        backRuns: [for (final _ in back) const <InlineRunModel>[]],
        answerSteps: [
          for (var i = 0; i < back.length; i++)
            ReviewAnswerLineModel(backFrom: i, backTo: i + 1),
        ],
        backUnits: [
          ReviewDiagramModel(
            src: source,
            width: 188,
            height: 114,
            alt: 'flowchart LR\n A-->B',
          ),
        ],
        reshaped: false,
        note: const [],
        images: const [],
        imagesBack: const [],
      );
      final state = ReviewStateModel(
        card: card,
        mode: ReviewMode.lineByLine,
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
        dueLeft: 0,
        newLeft: 0,
      );

      await tester.pumpWidget(
        MaterialApp(
          theme: alixDark(),
          home: Scaffold(
            body: ReviewCardView(
              state: state,
              revealed: false,
              revealedLines: card.back.length,
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
              serverLive: true,
              tutorCard: null,
              verdictGrade: ReviewGrade.fail,
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

      expect(find.byType(Image), findsOneWidget, reason: '$back');
      expect(
        find.bySemanticsLabel('flowchart LR\n A-->B'),
        findsOneWidget,
        reason: '$back',
      );
    }
  });

  testWidgets('a front fence renders the resolved diagram unit', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    final source = File('assets/icon/alix-192.png').absolute.path;
    final card = ReviewCardModel(
      front: 'Topic\n```mermaid\nflowchart LR\n A-->B\n```',
      frontRuns: const [],
      frontUnits: [
        ReviewSentenceModel(text: 'Topic', runs: const []),
        ReviewDiagramModel(
          src: source,
          width: 188,
          height: 114,
          alt: 'flowchart LR\n A-->B',
        ),
      ],
      context: const [],
      contextRuns: const [],
      back: const ['answer'],
      backRuns: const [[]],
      backUnits: const [],
      answerSteps: const [ReviewAnswerLineModel(backFrom: 0, backTo: 1)],
      reshaped: false,
      note: const [],
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
      dueLeft: 0,
      newLeft: 0,
    );

    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: Scaffold(
          body: ReviewCardView(
            state: state,
            revealed: false,
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
            serverLive: true,
            tutorCard: null,
            verdictGrade: ReviewGrade.fail,
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

    expect(find.byType(Image), findsOneWidget);
    expect(find.bySemanticsLabel('flowchart LR\n A-->B'), findsOneWidget);
    expect(find.textContaining('```'), findsNothing);
  });

  testWidgets('a masked front diagram reveals only its asked label text', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    final source = File('assets/icon/alix-192.png').absolute.path;
    ReviewCardModel card() => ReviewCardModel(
      front: 'Topic\n```mermaid\nflowchart LR\n  A[x] --> B[y]\n```',
      frontRuns: const [],
      context: const [],
      contextRuns: const [],
      frontUnits: [
        ReviewSentenceModel(text: 'Topic', runs: const []),
        ReviewDiagramModel(
          src: source,
          width: 188,
          height: 114,
          alt: 'diagram labels: …, …',
          regions: [
            ReviewRegionModel(
              role: ReviewRegionRole.asked,
              revealOnAnswer: true,
              x: 10,
              y: 50,
              width: 100,
              height: 40,
              unit: 'px',
            ),
          ],
          revealedAlt: 'diagram labels: …, Cache',
        ),
      ],
      back: const ['Cache'],
      backRuns: const [[]],
      backUnits: const [],
      answerSteps: const [ReviewAnswerLineModel(backFrom: 0, backTo: 1)],
      reshaped: false,
      note: const [],
      images: const [],
      imagesBack: const [],
    );
    ReviewStateModel state() => ReviewStateModel(
      card: card(),
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
      dueLeft: 0,
      newLeft: 0,
    );

    Widget app({required bool revealed}) => MaterialApp(
      theme: alixDark(),
      home: Scaffold(
        body: ReviewCardView(
          state: state(),
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
          serverLive: true,
          tutorCard: null,
          verdictGrade: ReviewGrade.fail,
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

    await tester.pumpWidget(app(revealed: false));
    expect(find.bySemanticsLabel('diagram labels: …, …'), findsOneWidget);
    expect(find.bySemanticsLabel('diagram labels: …, Cache'), findsNothing);
    expect(
      tester.widget<MaskedCardImage>(find.byType(MaskedCardImage)).height,
      114,
      reason:
          'masked diagrams use the manifest logical height, not the generic card-image height',
    );

    await tester.pumpWidget(app(revealed: true));
    expect(find.bySemanticsLabel('diagram labels: …, Cache'), findsOneWidget);
    expect(find.bySemanticsLabel('diagram labels: …, …'), findsNothing);
  });

  testWidgets('explain reveal renders the resolved diagram unit', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    final source = File('assets/icon/alix-192.png').absolute.path;
    final card = ReviewCardModel(
      front: 'Question',
      frontRuns: const [],
      context: const [],
      contextRuns: const [],
      back: const ['```mermaid', 'flowchart LR', ' A-->B', '```'],
      backRuns: const [[], [], [], []],
      answerSteps: const [ReviewAnswerLineModel(backFrom: 0, backTo: 1), ReviewAnswerLineModel(backFrom: 1, backTo: 2), ReviewAnswerLineModel(backFrom: 2, backTo: 3), ReviewAnswerLineModel(backFrom: 3, backTo: 4)],
      backUnits: [
        ReviewDiagramModel(
          src: source,
          width: 188,
          height: 114,
          alt: 'flowchart LR\n A-->B',
        ),
      ],
      reshaped: false,
      note: const [],
      images: const [],
      imagesBack: const [],
    );
    final state = ReviewStateModel(
      card: card,
      mode: ReviewMode.explain,
      depth: ReviewDepth.reconstruct,
      introducing: false,
      keypoints: const ['A reaches B'],
      finished: false,
      remaining: 1,
      reviews: 0,
      passed: 0,
      failed: 0,
      introduced: 0,
      partial: 0,
      canRestart: false,
      dueLeft: 0,
      newLeft: 0,
    );

    await tester.pumpWidget(
      MaterialApp(
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
            serverLive: true,
            tutorCard: null,
            verdictGrade: ReviewGrade.fail,
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

    expect(find.byType(Image), findsOneWidget);
    expect(find.bySemanticsLabel('flowchart LR\n A-->B'), findsOneWidget);
  });
}
