import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/walk/walk_controller.dart';
import 'package:alix_mobile/walk/walk_models.dart';
import 'package:alix_mobile/walk/walk_port.dart';

void main() {
  test('named transitions own server liveness, prediction, and grading', () {
    final port = _FakeWalkPort(_state(WalkPhaseModel.predict));
    final controller = WalkController(
      factory: _FakeWalkFactory([port]),
      deckPath: '/decks/trace.md',
      rootDir: '/decks',
      device: 'phone',
    );
    var notifications = 0;
    controller.addListener(() => notifications++);

    controller.setServerLive(true);
    controller.predict('my prediction');
    controller.grade(WalkGrade.got);

    expect(controller.serverLive, isTrue);
    expect(port.predictions, ['my prediction']);
    expect(port.grades, [WalkGrade.got]);
    expect(controller.state.phase, WalkPhaseModel.done);
    expect(notifications, 3);
  });

  test(
    'restart reports an open failure and can replace it with a new port',
    () {
      final recovered = _FakeWalkPort(_state(WalkPhaseModel.predict));
      final factory = _FakeWalkFactory([
        const WalkOpenFailure('not a trace'),
        recovered,
      ]);
      final controller = WalkController(
        factory: factory,
        deckPath: '/decks/facts.md',
        rootDir: '/decks',
      );
      expect(controller.openError, 'not a trace');

      var notifications = 0;
      controller.addListener(() => notifications++);
      controller.restart();

      expect(controller.openError, isNull);
      expect(controller.state.phase, WalkPhaseModel.predict);
      expect(factory.opens, 2);
      expect(notifications, 1);
    },
  );
}

WalkStateModel _state(WalkPhaseModel phase) {
  return WalkStateModel(
    phase: phase,
    description: 'how it works',
    descriptionRuns: const [],
    total: 1,
    current: 1,
    givens: const [],
    givenRuns: const [],
    points: const [],
    pointRuns: const [],
  );
}

class _FakeWalkFactory implements WalkPortFactory {
  _FakeWalkFactory(this.results);

  final List<Object> results;
  int opens = 0;

  @override
  WalkPort open({
    required String deckPath,
    required String rootDir,
    String? device,
  }) {
    final result = results[opens++];
    if (result case final WalkOpenFailure failure) throw failure;
    return result as WalkPort;
  }
}

class _FakeWalkPort implements WalkPort {
  _FakeWalkPort(this._state);

  WalkStateModel _state;
  final List<String> predictions = [];
  final List<WalkGrade> grades = [];

  @override
  WalkStateModel get state => _state;

  @override
  void predict(String text) {
    predictions.add(text);
    _state = _state.copyWith(phase: WalkPhaseModel.reveal, prediction: text);
  }

  @override
  WalkStateModel grade(WalkGrade grade) {
    grades.add(grade);
    _state = _state.copyWith(
      phase: WalkPhaseModel.done,
      summary: WalkSummaryModel(
        passed: 1,
        partly: 0,
        failed: 0,
        weak: [],
        total: 1,
      ),
    );
    return _state;
  }

  @override
  int? examCooldownMs(int nowMs) => null;

  @override
  void applyExamFailed(int nowMs) {}

  @override
  void applyExamPassed(int nowMs) {}
}
