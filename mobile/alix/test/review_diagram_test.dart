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
    'ReviewCardView renders supplied context math units for paired blocks '
    'and keeps unmatched openers literal',
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
      ReviewCardModel card(
        List<String> lines,
        List<ReviewContentUnitModel> units,
      ) {
        return ReviewCardModel(
          front: 'Topic',
          frontRuns: const [],
          context: lines,
          contextLeads: false,
          contextRuns: [
            for (final line in lines)
              [
                InlineRunModel(
                  text: line,
                  bold: false,
                  italic: false,
                  code: false,
                ),
              ],
          ],
          contextUnits: units,
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
            ),
          ),
        );
      }

      final unit = ReviewSentenceModel(text: 'x^2', runs: [mathRun]);
      for (final lines in [
        ['\$\$', 'x^2', '\$\$'],
        ['```math', 'x^2', '```'],
      ]) {
        await tester.pumpWidget(app(card(lines, [unit])));
        expect(find.bySemanticsLabel('x^2'), findsOneWidget, reason: '$lines');
        expect(find.byType(SvgPicture), findsOneWidget, reason: '$lines');
        expect(
          find.textContaining('math could not render', findRichText: true),
          findsNothing,
          reason: '$lines',
        );
        expect(find.textContaining('\$\$'), findsNothing, reason: '$lines');
        expect(find.textContaining('```'), findsNothing, reason: '$lines');
      }

      await tester.pumpWidget(app(card(['\$\$', 'x^2'], [])));
      expect(find.text('\$\$'), findsOneWidget);
      expect(find.text('x^2'), findsOneWidget);

      final fenceRows = [
        (
          'a longer context fence must not steal the following math unit',
          '````text',
          const ['before', '```', 'after'],
          '````',
        ),
        (
          'a five-tilde fence must not close on its interior triple run',
          '~~~~~',
          const ['before', '~~~', 'after'],
          '~~~~~',
        ),
        (
          'a backtick fence must not close on a tilde run',
          '```',
          const ['before', '~~~', 'after'],
          '```',
        ),
        (
          'a longer run of the same marker is a legal closer',
          '```text',
          const ['before'],
          '`````',
        ),
      ];
      for (final (label, opener, body, closer) in fenceRows) {
        await tester.pumpWidget(
          app(
            card(
              [opener, ...body, closer, '\$\$', 'x^2', '\$\$'],
              [ReviewCodeModel(body), unit],
            ),
          ),
        );
        expect(find.bySemanticsLabel('x^2'), findsOneWidget, reason: label);
      }
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
        contextLeads: false,
        contextRuns: const [],
        contextUnits: const [],
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

  testWidgets('a leading context fence renders the resolved diagram unit', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    final source = File('assets/icon/alix-192.png').absolute.path;
    final card = ReviewCardModel(
      front: 'Topic',
      frontRuns: const [],
      context: const ['```mermaid', 'flowchart LR', ' A-->B', '```'],
      contextLeads: true,
      contextRuns: const [[], [], [], []],
      contextUnits: [
        ReviewDiagramModel(
          src: source,
          width: 188,
          height: 114,
          alt: 'flowchart LR\n A-->B',
        ),
      ],
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
          ),
        ),
      ),
    );

    expect(find.byType(Image), findsOneWidget);
    expect(find.bySemanticsLabel('flowchart LR\n A-->B'), findsOneWidget);
    expect(find.textContaining('```'), findsNothing);
  });

  testWidgets('a masked context diagram reveals only its asked label text', (
    tester,
  ) async {
    final attempt = TextEditingController();
    addTearDown(attempt.dispose);
    final source = File('assets/icon/alix-192.png').absolute.path;
    ReviewCardModel card() => ReviewCardModel(
      front: 'Topic',
      frontRuns: const [],
      context: const ['```mermaid', 'flowchart LR', '  A[x] --> B[y]', '```'],
      contextLeads: true,
      contextRuns: const [[], [], [], []],
      contextUnits: [
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
      contextLeads: false,
      contextRuns: const [],
      contextUnits: const [],
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
          ),
        ),
      ),
    );

    expect(find.byType(Image), findsOneWidget);
    expect(find.bySemanticsLabel('flowchart LR\n A-->B'), findsOneWidget);
  });
}
