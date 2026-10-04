import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/trace/trace_controller.dart';
import 'package:alix_mobile/trace/trace_models.dart';
import 'package:alix_mobile/trace/trace_port.dart';

void main() {
  test('named transitions own server liveness, prediction, and grading', () {
    final port = _FakeTraceSessionPort(_state(TraceSessionPhaseModel.predict));
    final controller = TraceSessionController(
      factory: _FakeTraceSessionFactory([port]),
      deckPath: '/decks/trace.md',
      rootDir: '/decks',
      device: 'phone',
    );
    var notifications = 0;
    controller.addListener(() => notifications++);

    controller.setServerLive(true);
    controller.predict('my prediction');
    controller.grade(TraceSessionGrade.got);

    expect(controller.serverLive, isTrue);
    expect(port.predictions, ['my prediction']);
    expect(port.grades, [TraceSessionGrade.got]);
    expect(controller.state.phase, TraceSessionPhaseModel.done);
    expect(notifications, 3);
  });

  test(
    'restart reports an open failure and can replace it with a new port',
    () {
      final recovered = _FakeTraceSessionPort(_state(TraceSessionPhaseModel.predict));
      final factory = _FakeTraceSessionFactory([
        const TraceSessionOpenFailure('not a trace'),
        recovered,
      ]);
      final controller = TraceSessionController(
        factory: factory,
        deckPath: '/decks/facts.md',
        rootDir: '/decks',
      );
      expect(controller.openError, 'not a trace');

      var notifications = 0;
      controller.addListener(() => notifications++);
      controller.restart();

      expect(controller.openError, isNull);
      expect(controller.state.phase, TraceSessionPhaseModel.predict);
      expect(factory.opens, 2);
      expect(notifications, 1);
    },
  );
}

TraceSessionStateModel _state(TraceSessionPhaseModel phase) {
  return TraceSessionStateModel(
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

class _FakeTraceSessionFactory implements TraceSessionPortFactory {
  _FakeTraceSessionFactory(this.results);

  final List<Object> results;
  int opens = 0;

  @override
  TraceSessionPort open({
    required String deckPath,
    required String rootDir,
    String? device,
  }) {
    final result = results[opens++];
    if (result case final TraceSessionOpenFailure failure) throw failure;
    return result as TraceSessionPort;
  }
}

class _FakeTraceSessionPort implements TraceSessionPort {
  _FakeTraceSessionPort(this._state);

  TraceSessionStateModel _state;
  final List<String> predictions = [];
  final List<TraceSessionGrade> grades = [];

  @override
  TraceSessionStateModel get state => _state;

  @override
  void predict(String text) {
    predictions.add(text);
    _state = _state.copyWith(phase: TraceSessionPhaseModel.reveal, prediction: text);
  }

  @override
  TraceSessionStateModel grade(TraceSessionGrade grade) {
    grades.add(grade);
    _state = _state.copyWith(
      phase: TraceSessionPhaseModel.done,
      summary: TraceSessionSummaryModel(
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
