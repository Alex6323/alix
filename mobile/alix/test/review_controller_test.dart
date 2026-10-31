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
    controller.toggleChoice(2);
    controller.submitChoices();
    controller.check(const ['answer']);
    controller.openAttempt();
    controller.toggleKeypoint(0);

    expect(controller.serverLive, isTrue);
    expect(controller.revealed, isTrue);
    expect(controller.revealedLines, 1);
    expect(controller.choice?.chosen, 1);
    expect(controller.multiSelected, {2});
    expect(controller.multiChoice?.chosen, {2});
    expect(controller.checkFeedback?.passed, isTrue);
    expect(controller.attemptOpen, isTrue);
    expect(controller.tickedKeypoints, {0});
    expect(notifications, 9);
  });

  test('install resets card interaction state after introduce and grade', () {
    final first = _state();
    final second = _state(remaining: 1);
    final done = _state(remaining: 0, finished: true);
    final port = _FakeReviewPort(first)
      ..introduceResult = second
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
    controller.toggleChoice(1);
    controller.submitChoices();
    controller.introduce();

    expect(controller.state, same(second));
    expect(controller.revealed, isFalse);
    expect(controller.attemptOpen, isFalse);
    expect(controller.tickedKeypoints, isEmpty);
    expect(controller.multiSelected, isEmpty);
    expect(controller.multiChoice, isNull);

    controller.reveal();
    controller.grade(ReviewGrade.pass);
    expect(controller.state, same(done));
    expect(controller.revealed, isFalse);
    expect(port.grades, [ReviewGrade.pass]);
  });

  test('applying a card note refreshes it without hiding the answer', () {
    final note = ReviewNoteModel(
      badge: ReviewBadge.note,
      units: [ReviewSentenceModel(text: 'Tutor note', runs: const [])],
    );
    final port = _FakeReviewPort(_state())
      ..applyCardNoteResult = _state(note: [note]);
    final controller = ReviewController(
      factory: _FakeReviewFactory([port]),
      deckPath: '/decks/facts.md',
      rootDir: '/decks',
      depth: ReviewDepth.recall,
    );
    controller.reveal();
    var notifications = 0;
    controller.addListener(() => notifications++);

    controller.applyCardNote(id: 'card-one', notes: const ['Tutor note']);

    expect(controller.state.card?.note, [same(note)]);
    expect(controller.revealed, isTrue);
    expect(notifications, 1);
  });

  test('a hint opens only after the port accepts the current card', () {
    final port = _FakeReviewPort(
      _state(hints: [_hint('first'), _hint('second')]),
      tutor: _tutor('card-one'),
    )..openHintResult = false;
    final controller = ReviewController(
      factory: _FakeReviewFactory([port]),
      deckPath: '/decks/facts.md',
      rootDir: '/decks',
      depth: ReviewDepth.recall,
    );

    expect(
      controller.openHint(),
      isFalse,
      reason: 'a refused event opens nothing',
    );
    expect(controller.openedHints, 0);
    port.openHintResult = true;
    expect(
      controller.openHint(),
      isTrue,
      reason: 'the accepted event opens the first',
    );
    expect(controller.openedHints, 1);
    expect(port.openedHintIds, ['card-one', 'card-one']);

    controller.openNextHint();
    controller.openNextHint();
    expect(
      controller.openedHints,
      2,
      reason: 'the local stack stops at the authored count',
    );
    expect(
      controller.openHint(),
      isTrue,
      reason: 'reopening keeps the existing stack',
    );
    expect(
      port.openedHintIds,
      ['card-one', 'card-one'],
      reason: 'later hints and reopen do not record another event',
    );
  });

  test('the same card re-served after a skip has a fresh hint stack', () {
    final state = _state(hints: [_hint('first'), _hint('second')]);
    final port = _FakeReviewPort(state, tutor: _tutor('card-one'));
    final controller = ReviewController(
      factory: _FakeReviewFactory([port]),
      deckPath: '/decks/facts.md',
      rootDir: '/decks',
      depth: ReviewDepth.recall,
    );

    expect(controller.openHint(), isTrue);
    controller.openNextHint();
    expect(controller.openedHints, 2);
    controller.install(state);

    expect(
      controller.state,
      same(state),
      reason: 'the same card id was re-served',
    );
    expect(
      controller.openedHints,
      0,
      reason: 'the new attempt has no stale stack',
    );
    expect(controller.openHint(), isTrue);
    expect(
      port.openedHintIds,
      ['card-one', 'card-one'],
      reason: 'the new attempt records its own first open',
    );
  });

  test('a cram session opens crammed and stays crammed across a restart', () {
    final factory = _FakeReviewFactory([
      _FakeReviewPort(_state()),
      _FakeReviewPort(_state()),
    ]);
    final controller = ReviewController(
      factory: factory,
      deckPath: '/decks/facts.md',
      rootDir: '/decks',
      depth: ReviewDepth.recall,
      cram: true,
    );
    controller.restart();

    expect(factory.crams, [true, true]);
  });

  test('a session opens uncrammed and introducing unless asked', () {
    final factory = _FakeReviewFactory([_FakeReviewPort(_state())]);
    ReviewController(
      factory: factory,
      deckPath: '/decks/facts.md',
      rootDir: '/decks',
      depth: ReviewDepth.recall,
    );

    expect(factory.crams, [false]);
    expect(factory.skips, [false]);
  });

  test('a session that skips introduction keeps skipping across a restart', () {
    final factory = _FakeReviewFactory([
      _FakeReviewPort(_state()),
      _FakeReviewPort(_state()),
    ]);
    final controller = ReviewController(
      factory: factory,
      deckPath: '/decks/facts.md',
      rootDir: '/decks',
      depth: ReviewDepth.recall,
      skipIntroduction: true,
    );
    controller.restart();

    expect(factory.skips, [true, true]);
    expect(factory.crams, [false, false]);
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

ReviewStateModel _state({
  int remaining = 2,
  bool finished = false,
  List<ReviewNoteModel> note = const [],
  List<List<ReviewContentUnitModel>> hints = const [],
}) {
  return ReviewStateModel(
    card: ReviewCardModel(
      front: 'question',
      frontRuns: const [],
      back: const ['answer'],
      backRuns: const [[]],
      backUnits: const [],
      answerSteps: const [ReviewAnswerLineModel(backFrom: 0, backTo: 1)],
      reshaped: false,
      note: note,
      hints: hints,
      images: const [],
      imagesBack: const [],
    ),
    mode: ReviewMode.explain,
    depth: ReviewDepth.recall,
    introducing: false,
    choices: const ['wrong', 'right'],
    choiceRuns: const [[], []],
    keypoints: const ['point'],
    keypointRuns: const [[]],
    finished: finished,
    remaining: remaining,
    reviews: 0,
    passed: 0,
    failed: 0,
    introduced: 0,
    partial: 0,
    canRestart: true,
    dueLeft: remaining,
    newLeft: 0,
  );
}

List<ReviewContentUnitModel> _hint(String text) => [
  ReviewSentenceModel(text: text, runs: const []),
];

ReviewTutorCardModel _tutor(String id) => ReviewTutorCardModel(
  id: id,
  deckId: 'deck-facts',
  subject: 'facts.md',
  front: 'question',
  back: const ['answer'],
  line: 1,
);

class _FakeReviewFactory implements ReviewPortFactory {
  _FakeReviewFactory(this.results);

  final List<Object> results;
  final List<bool> crams = [];
  final List<bool> skips = [];
  int opens = 0;

  @override
  ReviewPort open({
    required String deckPath,
    required String rootDir,
    ReviewDepth? depth,
    bool cram = false,
    bool skipIntroduction = false,
    String? device,
  }) {
    crams.add(cram);
    skips.add(skipIntroduction);
    final result = results[opens++];
    if (result case final ReviewOpenFailure failure) throw failure;
    return result as ReviewPort;
  }
}

class _FakeReviewPort implements ReviewPort {
  _FakeReviewPort(this._state, {this.tutor});

  ReviewStateModel _state;
  final ReviewTutorCardModel? tutor;
  ReviewStateModel? introduceResult;
  ReviewStateModel? gradeResult;
  ReviewStateModel? applyCardNoteResult;
  bool openHintResult = true;
  final List<ReviewGrade> grades = [];
  final List<String> openedHintIds = [];

  @override
  ReviewStateModel get state => _state;

  @override
  ReviewStateModel introduce() => _state = introduceResult ?? _state;

  @override
  ReviewStateModel skip() => _state;

  @override
  bool openHint(String cardId) {
    openedHintIds.add(cardId);
    return openHintResult;
  }

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
  ReviewMultiChoiceFeedbackModel? chooseMulti(List<int> chosen) {
    return ReviewMultiChoiceFeedbackModel(
      chosen: chosen,
      correct: const [2],
      passed: chosen.length == 1 && chosen.single == 2,
    );
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
  ReviewTutorCardModel? get tutorCard => tutor;

  @override
  ReviewGrade keypointGrade({required int covered, required int total}) {
    return covered == total ? ReviewGrade.pass : ReviewGrade.partial;
  }

  @override
  void applyCardNote({required String id, required List<String> notes}) {
    _state = applyCardNoteResult ?? _state;
  }

  @override
  void applyExamPassed(int nowMs) {}

  @override
  int applyRemediation({required String cardsText}) => 0;

  @override
  String mintTutorCard({required String front, required List<String> back}) =>
      'minted';

  @override
  String recordMintedCard({required String block}) => 'recorded';
}
