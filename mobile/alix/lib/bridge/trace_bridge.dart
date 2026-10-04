import 'package:alix_mobile/bridge/bridge_error.dart';
import 'package:alix_mobile/bridge/inline_run_bridge.dart';
import 'package:alix_mobile/src/rust/api/review.dart' as bridge;
import 'package:alix_mobile/trace/trace_models.dart';
import 'package:alix_mobile/trace/trace_port.dart';

class TraceSessionBridgeFactory implements TraceSessionPortFactory {
  const TraceSessionBridgeFactory();

  @override
  TraceSessionPort open({
    required String deckPath,
    required String rootDir,
    String? device,
  }) {
    try {
      return TraceSessionBridgePort(
        bridge.TraceSession.open(
          deckPath: deckPath,
          rootDir: rootDir,
          device: device,
        ),
      );
    } catch (error) {
      throw TraceSessionOpenFailure(bridgeErrorText(error));
    }
  }
}

class TraceSessionBridgePort implements TraceSessionPort {
  TraceSessionBridgePort(this._session);

  final bridge.TraceSession _session;

  @override
  TraceSessionStateModel get state => _stateFromBridge(_session.state());

  @override
  void predict(String text) => _session.predict(text: text);

  @override
  TraceSessionStateModel grade(TraceSessionGrade grade) {
    return _stateFromBridge(_session.grade(delta: _gradeToBridge(grade)));
  }

  @override
  int? examCooldownMs(int nowMs) {
    return _session.examCooldownMs(nowMs: BigInt.from(nowMs))?.toInt();
  }

  @override
  void applyExamPassed(int nowMs) {
    _session.applyExamPassed(nowMs: BigInt.from(nowMs));
  }

  @override
  void applyExamFailed(int nowMs) {
    _session.applyExamFailed(nowMs: BigInt.from(nowMs));
  }
}

TraceSessionStateModel _stateFromBridge(bridge.TraceSessionState state) {
  return TraceSessionStateModel(
    phase: switch (state.phase) {
      bridge.TraceSessionPhase.predict => TraceSessionPhaseModel.predict,
      bridge.TraceSessionPhase.reveal => TraceSessionPhaseModel.reveal,
      bridge.TraceSessionPhase.done => TraceSessionPhaseModel.done,
    },
    description: state.description,
    descriptionRuns: inlineRunsFromBridge(state.descriptionRuns),
    source: state.source,
    total: state.total,
    current: state.current,
    prompt: state.prompt,
    promptRuns: state.promptRuns == null
        ? null
        : inlineRunsFromBridge(state.promptRuns!),
    givens: state.givens,
    givenRuns: [for (final runs in state.givenRuns) inlineRunsFromBridge(runs)],
    locator: state.locator,
    prediction: state.prediction,
    excerpt: state.excerpt == null
        ? null
        : TraceSessionExcerptModel(
            path: state.excerpt!.path,
            lines: [
              for (final line in state.excerpt!.lines)
                TraceSessionLineModel(number: line.n, text: line.text),
            ],
            truncated: state.excerpt!.truncated,
          ),
    excerptError: state.excerptError,
    points: state.points,
    pointRuns: [for (final runs in state.pointRuns) inlineRunsFromBridge(runs)],
    note: state.note,
    noteRuns: state.noteRuns == null
        ? null
        : inlineRunsFromBridge(state.noteRuns!),
    summary: state.summary == null
        ? null
        : TraceSessionSummaryModel(
            passed: state.summary!.passed,
            partly: state.summary!.partly,
            failed: state.summary!.failed,
            weak: state.summary!.weak,
            total: state.summary!.total,
          ),
    saveError: state.saveError,
  );
}

bridge.TraceSessionDelta _gradeToBridge(TraceSessionGrade grade) {
  return switch (grade) {
    TraceSessionGrade.missed => bridge.TraceSessionDelta.missed,
    TraceSessionGrade.partly => bridge.TraceSessionDelta.partly,
    TraceSessionGrade.got => bridge.TraceSessionDelta.got,
  };
}
