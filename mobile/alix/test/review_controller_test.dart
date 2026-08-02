import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_controller.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/review_port.dart';

void main() {
  test('every per-card mutation has a named controller transition', () {
    final port = _FakeReviewPort(_state());
    final controller = ReviewController(
      factory: _FakeReviewFactory([port]),
      deckPath: '/decks/facts.md',
      rootDir: '/decks',
      depth: ReviewDepth.recall,
      device: 'phone',
    );
    var notifications = 0;
    controller.addListener(() => notifications++);

    controller.setServerLive(true);
    controller.reveal();
    controller.revealNextLine();
    controller.choose(1);
    controller.check(const ['answer']);
    controller.openAttempt();
    controller.toggleKeypoint(0);
    controller.dismissForeignWriter();

    expect(controller.serverLive, isTrue);
    expect(controller.revealed, isTrue);
    expect(controller.revealedLines, 1);
    expect(controller.choice?.chosen, 1);
    expect(controller.checkFeedback?.passed, isTrue);
    expect(controller.attemptOpen, isTrue);
    expect(controller.tickedKeypoints, {0});
    expect(controller.foreignWriter, isNull);
    expect(notifications, 8);
  });

  test('install resets card interaction state after acquire and grade', () {
    final first = _state();
    final second = _state(remaining: 1);
    final done = _state(remaining: 0, finished: true);
    final port = _FakeReviewPort(first)
      ..acquireResult = second
      ..gradeResult = done;
    final controller = ReviewController(
      factory: _FakeReviewFactory([port]),
      deckPath: '/decks/facts.md',
      rootDir: '/decks',
      depth: ReviewDepth.recall,
    );

    controller.reveal();
    controller.openAttempt();
    controller.toggleKeypoint(0);
    controller.acquire();

    expect(controller.state, same(second));
    expect(controller.revealed, isFalse);
    expect(controller.attemptOpen, isFalse);
    expect(controller.tickedKeypoints, isEmpty);

    controller.reveal();
    controller.grade(ReviewGrade.pass);
    expect(controller.state, same(done));
    expect(controller.revealed, isFalse);
    expect(port.grades, [ReviewGrade.pass]);
  });

  test(
    'restart reports an open failure and can replace it with a new port',
    () {
      final recovered = _FakeReviewPort(_state());
      final factory = _FakeReviewFactory([
        const ReviewOpenFailure('not a fact deck'),
        recovered,
      ]);
      final controller = ReviewController(
        factory: factory,
        deckPath: '/decks/trace.md',
        rootDir: '/decks',
        depth: ReviewDepth.recall,
      );
      expect(controller.openError, 'not a fact deck');

      var notifications = 0;
      controller.addListener(() => notifications++);
      controller.restart();

      expect(controller.openError, isNull);
      expect(controller.state.remaining, 2);
      expect(factory.opens, 2);
      expect(notifications, 1);
    },
  );
}

ReviewStateModel _state({int remaining = 2, bool finished = false}) {
  return ReviewStateModel(
    card: ReviewCardModel(
      front: 'question',
      frontRuns: const [],
      context: const [],
      contextRuns: const [],
      back: const ['answer'],
      backRuns: const [[]],
      backUnits: const [],
      reshaped: false,
      note: const [],
      images: const [],
      imagesBack: const [],
    ),
    mode: ReviewMode.explain,
    depth: ReviewDepth.recall,
    acquire: false,
    choices: const ['wrong', 'right'],
    choiceRuns: const [[], []],
    keypoints: const ['point'],
    keypointRuns: const [[]],
    finished: finished,
    remaining: remaining,
    reviews: 0,
    passed: 0,
    failed: 0,
    acquired: 0,
    recognized: 0,
    recognizePartly: 0,
    recognizeMissed: 0,
    canRestart: true,
    promotable: false,
    dueLeft: remaining,
    newLeft: 0,
  );
}

class _FakeReviewFactory implements ReviewPortFactory {
  _FakeReviewFactory(this.results);

  final List<Object> results;
  int opens = 0;

  @override
  ReviewPort open({
    required String deckPath,
    required String rootDir,
    ReviewDepth? depth,
    String? device,
  }) {
    final result = results[opens++];
    if (result case final ReviewOpenFailure failure) throw failure;
    return result as ReviewPort;
  }
}

class _FakeReviewPort implements ReviewPort {
  _FakeReviewPort(this._state);

  ReviewStateModel _state;
  ReviewStateModel? acquireResult;
  ReviewStateModel? gradeResult;
  final List<ReviewGrade> grades = [];

  @override
  ReviewStateModel get state => _state;

  @override
  ReviewForeignWriterModel? get foreignWriter =>
      const ReviewForeignWriterModel(device: 'laptop', ageMs: 1000);

  @override
  ReviewStateModel acquire() => _state = acquireResult ?? _state;

  @override
  ReviewCheckFeedbackModel? check(List<String> lines) {
    return ReviewCheckFeedbackModel(
      results: [
        ReviewTypedResultModel(
          input: lines.single,
          expected: 'answer',
          passed: true,
        ),
      ],
      passed: true,
    );
  }

  @override
  ReviewChoiceFeedbackModel? choose(int chosen) {
    return ReviewChoiceFeedbackModel(chosen: chosen, correct: 1, passed: true);
  }

  @override
  ReviewCrumbModel? crumb(int nowMs) => null;

  @override
  bool get deckHasExam => false;

  @override
  ReviewStateModel grade(ReviewGrade grade) {
    grades.add(grade);
    return _state = gradeResult ?? _state;
  }

  @override
  ReviewTutorCardModel? get tutorCard => null;

  @override
  ReviewGrade keypointGrade({required int covered, required int total}) {
    return covered == total ? ReviewGrade.pass : ReviewGrade.partial;
  }

  @override
  void applyCardNote({required int line, required List<String> notes}) {}

  @override
  void applyExamPassed(int nowMs) {}

  @override
  int applyRemediation({required String cardsText, required int nowMs}) => 0;

  @override
  String mintTutorCard({
    required String front,
    required List<String> back,
    required int nowMs,
  }) => 'minted';
}
