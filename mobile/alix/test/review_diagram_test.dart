import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/masked_image.dart';
import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/theme.dart';

void main() {
  testWidgets('complete line reveal renders the resolved diagram unit', (
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
            checkFeedback: null,
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
            checkFeedback: null,
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
          checkFeedback: null,
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
      reason: 'masked diagrams use the manifest logical height, not the generic card-image height',
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
            checkFeedback: null,
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
