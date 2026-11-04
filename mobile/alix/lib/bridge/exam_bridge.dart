import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/api/exam.dart' as bridge;

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
      sections: [
        for (final section in exam.sections)
          bridge.ExamSectionOutcome(cards: section.cards, pass: section.pass),
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
