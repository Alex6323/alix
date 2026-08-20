import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

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
