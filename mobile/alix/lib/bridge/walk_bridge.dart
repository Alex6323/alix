import 'package:flutter_rust_bridge/flutter_rust_bridge.dart'
    show AnyhowException;

import 'package:alix_mobile/bridge/inline_run_bridge.dart';
import 'package:alix_mobile/src/rust/api/review.dart' as bridge;
import 'package:alix_mobile/walk/walk_models.dart';
import 'package:alix_mobile/walk/walk_port.dart';

class WalkBridgeFactory implements WalkPortFactory {
  const WalkBridgeFactory();

  @override
  WalkPort open({
    required String deckPath,
    required String rootDir,
    String? device,
  }) {
    try {
      return WalkBridgePort(
        bridge.WalkSession.open(
          deckPath: deckPath,
          rootDir: rootDir,
          device: device,
        ),
      );
    } catch (error) {
      throw WalkOpenFailure(
        error is AnyhowException ? error.message : '$error',
      );
    }
  }
}

class WalkBridgePort implements WalkPort {
  WalkBridgePort(this._session);

  final bridge.WalkSession _session;

  @override
  WalkStateModel get state => _stateFromBridge(_session.state());

  @override
  void predict(String text) => _session.predict(text: text);

  @override
  WalkStateModel grade(WalkGrade grade) {
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

WalkStateModel _stateFromBridge(bridge.WalkState state) {
  return WalkStateModel(
    phase: switch (state.phase) {
      bridge.WalkPhase.predict => WalkPhaseModel.predict,
      bridge.WalkPhase.reveal => WalkPhaseModel.reveal,
      bridge.WalkPhase.done => WalkPhaseModel.done,
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
        : WalkExcerptModel(
            path: state.excerpt!.path,
            lines: [
              for (final line in state.excerpt!.lines)
                WalkLineModel(number: line.n, text: line.text),
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
        : WalkSummaryModel(
            passed: state.summary!.passed,
            partly: state.summary!.partly,
            failed: state.summary!.failed,
            weak: state.summary!.weak,
            total: state.summary!.total,
          ),
    saveError: state.saveError,
  );
}

bridge.WalkDelta _gradeToBridge(WalkGrade grade) {
  return switch (grade) {
    WalkGrade.missed => bridge.WalkDelta.missed,
    WalkGrade.partly => bridge.WalkDelta.partly,
    WalkGrade.got => bridge.WalkDelta.got,
  };
}
