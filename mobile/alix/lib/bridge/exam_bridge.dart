import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/api/exam.dart' as bridge;

String? examIdeasLoad({
  required String deckPath,
  required String rootDir,
}) => bridge.examIdeasLoad(deckPath: deckPath, rootDir: rootDir);

void examIdeasSave({
  required String deckPath,
  required String rootDir,
  required String json,
}) => bridge.examIdeasSave(deckPath: deckPath, rootDir: rootDir, json: json);

List<String> examAnswersLoad({
  required String deckPath,
  required String rootDir,
  required List<String> questions,
}) => bridge.examAnswersLoad(
  deckPath: deckPath,
  rootDir: rootDir,
  questions: questions,
);

void examAnswersSave({
  required String deckPath,
  required String rootDir,
  required List<String> questions,
  required List<String> answers,
}) => bridge.examAnswersSave(
  deckPath: deckPath,
  rootDir: rootDir,
  questions: questions,
  answers: answers,
);

void examAnswersClear({
  required String deckPath,
  required String rootDir,
}) => bridge.examAnswersClear(deckPath: deckPath, rootDir: rootDir);

/// A sitting's starting state, evaluated once against the phone's store:
/// the cards to ask about (null for a full sitting), whether the review
/// condition holds, and the evidence stamp a standing records.
class ExamSittingStart {
  const ExamSittingStart({
    required this.cards,
    required this.eligible,
    required this.sourced,
    required this.unsourcedCards,
  });

  final List<String>? cards;
  final bool eligible;
  final String sourced;
  final List<String> unsourcedCards;
}

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
    cards: start.cards,
    eligible: start.eligible,
    sourced: start.stamp.sourced,
    unsourcedCards: start.stamp.unsourcedCards,
  );
}

/// The deck's standing after the sitting, worded by the core, or null.
String? applyExamResult({
  required String deckPath,
  required String rootDir,
  required RemoteExam exam,
  required ExamSittingStart start,
  required BigInt nowMs,
  String? device,
}) {
  final verdicts = exam.verdicts;
  final applied = bridge.applyExamResult(
    deckPath: deckPath,
    rootDir: rootDir,
    outcome: bridge.ExamOutcome(
      topics: [
        for (final topic in exam.topics)
          bridge.ExamTopicOutcome(
            topicId: topic.topicId,
            fingerprint: topic.fingerprint,
            pass: topic.pass,
          ),
      ],
      full: exam.full,
      pass: verdicts?.pass ?? 0,
      partial: verdicts?.partial ?? 0,
      total: verdicts?.total ?? 0,
      eligible: start.eligible,
      stamp: bridge.ExamStamp(
        sourced: start.sourced,
        unsourcedCards: start.unsourcedCards,
      ),
    ),
    nowMs: nowMs,
    device: device,
  );
  return applied.standing;
}

int applyExamRemediation({
  required String deckPath,
  required String rootDir,
  required String topicId,
  required String cardsText,
  String? device,
}) => bridge.applyExamRemediation(
  deckPath: deckPath,
  rootDir: rootDir,
  topicId: topicId,
  cardsText: cardsText,
  device: device,
);
