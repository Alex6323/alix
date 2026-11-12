import 'package:alix_mobile/bridge/bridge_error.dart';
import 'package:alix_mobile/bridge/inline_run_bridge.dart';
import 'package:alix_mobile/bridge/review_bridge.dart';
import 'package:alix_mobile/review/review_models.dart';
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
    bool cram = false,
  }) {
    try {
      return WalkBridgePort(
        bridge.WalkSession.open(
          deckPath: deckPath,
          rootDir: rootDir,
          device: device,
          cram: cram,
        ),
      );
    } catch (error) {
      throw WalkOpenFailure(bridgeErrorText(error));
    }
  }

  @override
  WalkPort openTutorial({required String baseDir, String? device}) {
    try {
      return WalkBridgePort(
        bridge.WalkSession.openTutorial(baseDir: baseDir, device: device),
      );
    } catch (error) {
      throw WalkOpenFailure(bridgeErrorText(error));
    }
  }
}

class WalkBridgePort implements WalkPort {
  WalkBridgePort(this._session);

  final bridge.WalkSession _session;

  @override
  WalkStateModel get state => _stateFromBridge(_session.state());

  @override
  bool get isTutorial => _session.isTutorial();

  @override
  ReviewTutorCardModel? get tutorCard {
    final tutor = _session.tutorCard();
    return tutor == null
        ? null
        : ReviewTutorCardModel(
            id: tutor.id,
            deckId: tutor.deckId,
            subject: tutor.subject,
            front: tutor.front,
            back: tutor.back,
            at: tutor.at,
            line: tutor.line.toInt(),
          );
  }

  @override
  ReviewChoiceFeedbackModel? choose(int chosen) {
    final choice = _session.choose(chosen: chosen);
    return choice == null
        ? null
        : ReviewChoiceFeedbackModel(
            chosen: choice.chosen.toInt(),
            correct: choice.correct.toInt(),
            passed: choice.passed,
          );
  }

  @override
  ReviewMultiChoiceFeedbackModel? chooseMulti(List<int> chosen) {
    final feedback = _session.chooseMulti(chosen: chosen);
    return feedback == null
        ? null
        : ReviewMultiChoiceFeedbackModel(
            chosen: [for (final index in feedback.chosen) index.toInt()],
            correct: [for (final index in feedback.correct) index.toInt()],
            passed: feedback.passed,
          );
  }

  @override
  WalkStateModel reveal() => _stateFromBridge(_session.reveal());

  @override
  WalkStateModel grade(ReviewGrade grade) =>
      _stateFromBridge(_session.grade(grade: gradeToBridge(grade)));

  @override
  bool openHint(String cardId) => _session.openHint(cardId: cardId);

  @override
  WalkStateModel restart() => _stateFromBridge(_session.restart());

  @override
  String mintTutorCard({required String front, required List<String> back}) {
    return _session.mintTutorCard(front: front, back: back);
  }

  @override
  String recordMintedCard({required String block}) {
    return _session.recordMintedCard(block: block);
  }

  @override
  void applyCardNote({required String id, required List<String> notes}) {
    _session.applyCardNote(id: id, notes: notes);
  }
}

WalkStateModel _stateFromBridge(bridge.WalkState state) {
  return WalkStateModel(
    phase: switch (state.phase) {
      bridge.WalkPhase.front => WalkPhase.front,
      bridge.WalkPhase.answer => WalkPhase.answer,
      bridge.WalkPhase.done => WalkPhase.done,
    },
    card: state.card == null ? null : reviewCardFromBridge(state.card!),
    mode: reviewModeFromBridge(state.mode),
    input: reviewInputFromBridge(state.input),
    choices: state.choices,
    choiceRuns: state.choiceRuns == null
        ? null
        : [for (final runs in state.choiceRuns!) inlineRunsFromBridge(runs)],
    choicesMultiple: state.choicesMultiple,
    topicFirst: state.topicFirst,
    position: state.position,
    total: state.total,
    label: state.label,
    pick: switch (state.pick) {
      bridge.WalkPick.correct => true,
      bridge.WalkPick.wrong => false,
      null => null,
    },
    saveError: state.saveError,
  );
}
