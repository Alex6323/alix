import 'dart:convert';

import 'package:flutter/foundation.dart' show visibleForTesting;

import 'package:alix_mobile/bridge/bridge_error.dart';
import 'package:alix_mobile/exam_sittings.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/api/exam.dart' as bridge;

String? examIdeasLoad({
  required String deckPath,
  required String rootDir,
}) => bridge.examIdeasLoad(deckPath: deckPath, rootDir: rootDir);

ExamSittingStart examSittingStart({
  required String deckPath,
  required String rootDir,
  required bool needsWorkOnly,
  required BigInt nowMs,
}) {
  final start = bridge.examSittingStart(
    deckPath: deckPath,
    rootDir: rootDir,
    needsWorkOnly: needsWorkOnly,
    nowMs: nowMs,
  );
  return ExamSittingStart(
    deckId: start.deckId,
    cards: start.cards,
    eligible: start.eligible,
    sourced: start.stamp.sourced,
    unsourcedCards: start.stamp.unsourcedCards,
    unsourcedTopics: start.unsourcedTopics,
  );
}

String? examDeckId({required String deckPath}) => bridge.examDeckId(deckPath: deckPath);

String? heldExamSitting(String rootDir, String deckId) =>
    bridge.heldExamSitting(rootDir: rootDir, deckId: deckId);

void heldExamReceive(StartedSitting record, String carriedBody) => bridge.heldExamReceive(
  rootDir: record.root,
  carriedJson: carriedBody,
  start: bridge.ExamStart(
    deckId: record.start.deckId,
    cards: record.start.cards,
    eligible: record.start.eligible,
    stamp: bridge.ExamStamp(sourced: record.start.sourced, unsourcedCards: record.start.unsourcedCards),
    unsourcedTopics: record.start.unsourcedTopics,
  ),
);

@visibleForTesting
int heldExamViewReads = 0;

HeldExamView? heldExamView({required String rootDir, required String deckId}) {
  heldExamViewReads++;
  final json = bridge.heldExamView(rootDir: rootDir, deckId: deckId);
  return json == null ? null : HeldExamView.fromJson(jsonDecode(json) as Map<String, dynamic>);
}

void heldExamRemove({required String rootDir, required String deckId}) =>
    bridge.heldExamRemove(rootDir: rootDir, deckId: deckId);

const answerTooLongText = 'this answer is too long to send';

class AnswerTooLong implements Exception {
  const AnswerTooLong();

  @override
  String toString() => answerTooLongText;
}

class HeldExamStanding {
  const HeldExamStanding({required this.text, required this.examChanged});

  final String text;
  final bool examChanged;
}

class HeldExamOutcome {
  const HeldExamOutcome({required this.recorded, this.standing});

  final bool recorded;
  final HeldExamStanding? standing;
}

class HeldExamResult {
  const HeldExamResult({
    required this.grades,
    required this.topics,
    required this.verdicts,
    required this.mastered,
  });

  final List<RemoteExamGrade> grades;
  final List<RemoteExamTopic> topics;
  final ExamVerdicts verdicts;
  final bool mastered;
}

class HeldExamView {
  const HeldExamView({
    required this.sitting,
    required this.deck,
    required this.strictness,
    required this.full,
    required this.questions,
    required this.questionsUnits,
    required this.answers,
    required this.locked,
    required this.unsourcedTopics,
    required this.result,
    required this.outcome,
    required this.remediated,
    required this.canRemediate,
  });

  final String sitting;
  final String deck;
  final String strictness;
  final bool full;
  final List<String> questions;
  final List<List<ReviewContentUnitModel>> questionsUnits;
  final List<String> answers;
  final List<bool> locked;
  final List<String> unsourcedTopics;
  final HeldExamResult? result;
  final HeldExamOutcome? outcome;
  final Map<String, int> remediated;
  final bool canRemediate;

  static HeldExamView fromJson(Map<String, dynamic> json) => HeldExamView(
    sitting: _required<String>(json, 'sitting'),
    deck: _required<String>(json, 'deck'),
    strictness: json['strictness'] is String ? json['strictness'] as String : '',
    full: json['full'] == true,
    questions: _requiredList<String>(json, 'questions'),
    questionsUnits: [
      if (json['questions_units'] case final List units)
        for (final unit in units) contentUnitsFromJson(unit),
    ],
    answers: _requiredList<String>(json, 'answers'),
    locked: _requiredList<bool>(json, 'locked'),
    unsourcedTopics: [
      if (json['unsourced_topics'] case final List topics) ...topics.whereType<String>(),
    ],
    result: _result(json['result']),
    outcome: _outcome(json['outcome']),
    remediated: {
      if (json['remediated'] case final Map remediated)
        for (final MapEntry(:key, :value) in remediated.entries)
          if (key is String && value is int) key: value,
    },
    canRemediate: _required<bool>(json, 'can_remediate'),
  );
}

T _required<T>(Map<String, dynamic> json, String key) {
  final value = json[key];
  if (value is! T) throw FormatException('the held exam view has no $key of type $T');
  return value;
}

List<T> _requiredList<T>(Map<String, dynamic> json, String key) {
  final value = json[key];
  if (value is! List || value.any((item) => item is! T)) {
    throw FormatException('the held exam view has no $key list of $T');
  }
  return value.cast<T>();
}

HeldExamResult? _result(dynamic json) {
  if (json is! Map) return null;
  final verdicts = ExamVerdicts.fromJson(json['verdicts']);
  if (verdicts == null) throw const FormatException('the held exam result has no verdicts');
  return HeldExamResult(
    grades: [
      if (json['grades'] case final List grades)
        for (final grade in grades) ?RemoteExamGrade.fromJson(grade),
    ],
    topics: [
      if (json['topics'] case final List topics)
        for (final topic in topics) ?RemoteExamTopic.fromJson(topic),
    ],
    verdicts: verdicts,
    mastered: json['mastered'] == true,
  );
}

HeldExamOutcome? _outcome(dynamic json) {
  if (json is! Map) return null;
  final standing = json['standing'];
  return HeldExamOutcome(
    recorded: json['recorded'] == true,
    standing: standing is Map && standing['text'] is String
        ? HeldExamStanding(text: standing['text'] as String, examChanged: standing['exam_changed'] == true)
        : null,
  );
}

abstract class HeldExamAccess {
  HeldExamView? view();

  void saveAnswers(List<String> answers, List<bool> locked);

  bool materialChanged();

  String gradeRequest();

  void storeResult(String replyBody);

  HeldExamView apply();

  String? remediationRequest();

  int storeRemediation(String replyBody);

  void remove();
}

class BridgeHeldExam implements HeldExamAccess {
  BridgeHeldExam({
    required this.deckPath,
    required this.deckId,
    required this.rootDir,
    this.device,
    int Function()? clock,
  }) : _clock = clock ?? (() => DateTime.now().millisecondsSinceEpoch);

  final String deckPath;
  final String deckId;
  final String rootDir;
  final String? device;
  final int Function() _clock;

  @override
  HeldExamView? view() {
    final json = bridge.heldExamView(rootDir: rootDir, deckId: deckId);
    return json == null ? null : HeldExamView.fromJson(jsonDecode(json) as Map<String, dynamic>);
  }

  @override
  void saveAnswers(List<String> answers, List<bool> locked) => _tooLongAware(
    () => bridge.heldExamSaveAnswers(rootDir: rootDir, deckId: deckId, answers: answers, locked: locked),
  );

  @override
  bool materialChanged() => bridge.heldExamMaterialChanged(deckPath: deckPath, rootDir: rootDir);

  @override
  String gradeRequest() => _tooLongAware(() => bridge.heldExamGradeRequest(rootDir: rootDir, deckId: deckId));

  @override
  void storeResult(String replyBody) => bridge.heldExamStoreResult(
    rootDir: rootDir,
    deckId: deckId,
    replyJson: replyBody,
    receivedMs: BigInt.from(_clock()),
  );

  @override
  HeldExamView apply() => HeldExamView.fromJson(
    jsonDecode(bridge.heldExamApply(deckPath: deckPath, rootDir: rootDir, device: device)) as Map<String, dynamic>,
  );

  @override
  String? remediationRequest() => bridge.heldExamRemediationRequest(rootDir: rootDir, deckId: deckId);

  @override
  int storeRemediation(String replyBody) => bridge.heldExamStoreRemediation(
    deckPath: deckPath,
    rootDir: rootDir,
    replyJson: replyBody,
    device: device,
  );

  @override
  void remove() => bridge.heldExamRemove(rootDir: rootDir, deckId: deckId);
}

T _tooLongAware<T>(T Function() call) {
  try {
    return call();
  } on Object catch (error) {
    if (bridgeErrorText(error) == answerTooLongText) throw const AnswerTooLong();
    rethrow;
  }
}
