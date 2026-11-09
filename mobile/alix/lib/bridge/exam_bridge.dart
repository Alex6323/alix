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

List<String>? examStartCards({
  required String deckPath,
  required String rootDir,
  required bool needsWorkOnly,
}) => bridge.examStartCards(
  deckPath: deckPath,
  rootDir: rootDir,
  needsWorkOnly: needsWorkOnly,
);

/// The deck's standing after the sitting, worded by the core, or null.
String? applyExamResult({
  required String deckPath,
  required String rootDir,
  required RemoteExam exam,
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
          bridge.ExamTopicOutcome(cards: topic.cards, pass: topic.pass),
      ],
      full: exam.full,
      pass: verdicts?.pass ?? 0,
      partial: verdicts?.partial ?? 0,
      total: verdicts?.total ?? 0,
    ),
    nowMs: nowMs,
    device: device,
  );
  return applied.standing;
}

int applyExamRemediation({
  required String deckPath,
  required String rootDir,
  required String cardsText,
  String? device,
}) => bridge.applyExamRemediation(
  deckPath: deckPath,
  rootDir: rootDir,
  cardsText: cardsText,
  device: device,
);
