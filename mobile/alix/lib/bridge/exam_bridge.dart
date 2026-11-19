import 'package:alix_mobile/exam_screen.dart' show ExamAnswers;
import 'package:alix_mobile/exam_sittings.dart';
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

ExamAnswers examAnswersLoad({
  required String deckPath,
  required String rootDir,
  required List<String> questions,
}) {
  final loaded = bridge.examAnswersLoad(
    deckPath: deckPath,
    rootDir: rootDir,
    questions: questions,
  );
  return (answers: loaded.answers, locked: loaded.locked);
}

void examAnswersSave({
  required String deckPath,
  required String rootDir,
  required List<String> questions,
  required ExamAnswers answers,
}) => bridge.examAnswersSave(
  deckPath: deckPath,
  rootDir: rootDir,
  questions: questions,
  answers: bridge.PhoneExamAnswers(answers: answers.answers, locked: answers.locked),
);

void examAnswersClear({
  required String deckPath,
  required String rootDir,
}) => bridge.examAnswersClear(deckPath: deckPath, rootDir: rootDir);

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

/// The deck's standing after the sitting, worded by the core, and whether
/// its material has changed since; null while the deck has no standing.
({String text, bool examChanged})? applyExamResult({
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
  final text = applied.examHighMark;
  if (text == null) return null;
  return (text: text, examChanged: applied.examChanged);
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
