import 'package:alix_mobile/shared/inline_models.dart';

enum ReviewDepth { recognize, recall, reconstruct }

enum ReviewMode { flip, typing, typeLine, choice, lineByLine, explain }

enum ReviewGrade { fail, partial, pass }

class ReviewImageModel {
  const ReviewImageModel({required this.src, this.alt});

  final String src;
  final String? alt;
}

class ReviewChecklistItemModel {
  ReviewChecklistItemModel({
    required this.checked,
    required this.text,
    required Iterable<InlineRunModel> runs,
  }) : runs = List.unmodifiable(runs);

  final bool checked;
  final String text;
  final List<InlineRunModel> runs;
}

sealed class ReviewNoteUnitModel {
  const ReviewNoteUnitModel();
}

class ReviewSentenceModel extends ReviewNoteUnitModel {
  ReviewSentenceModel({
    required this.text,
    required Iterable<InlineRunModel> runs,
  }) : runs = List.unmodifiable(runs);

  final String text;
  final List<InlineRunModel> runs;
}

class ReviewCodeModel extends ReviewNoteUnitModel {
  ReviewCodeModel(Iterable<String> lines) : lines = List.unmodifiable(lines);

  final List<String> lines;
}

class ReviewChecklistModel extends ReviewNoteUnitModel {
  ReviewChecklistModel(Iterable<ReviewChecklistItemModel> items)
    : items = List.unmodifiable(items);

  final List<ReviewChecklistItemModel> items;
}

class ReviewCardModel {
  ReviewCardModel({
    required this.front,
    required Iterable<InlineRunModel> frontRuns,
    Iterable<ReviewNoteUnitModel>? frontUnits,
    required Iterable<String> context,
    required Iterable<Iterable<InlineRunModel>> contextRuns,
    required Iterable<String> back,
    required Iterable<Iterable<InlineRunModel>> backRuns,
    required Iterable<ReviewNoteUnitModel> backUnits,
    required this.reshaped,
    required Iterable<ReviewNoteUnitModel> note,
    required Iterable<ReviewImageModel> images,
    required Iterable<ReviewImageModel> imagesBack,
  }) : frontRuns = List.unmodifiable(frontRuns),
       frontUnits = frontUnits == null ? null : List.unmodifiable(frontUnits),
       context = List.unmodifiable(context),
       contextRuns = _freezeRunLines(contextRuns),
       back = List.unmodifiable(back),
       backRuns = _freezeRunLines(backRuns),
       backUnits = List.unmodifiable(backUnits),
       note = List.unmodifiable(note),
       images = List.unmodifiable(images),
       imagesBack = List.unmodifiable(imagesBack);

  final String front;
  final List<InlineRunModel> frontRuns;
  final List<ReviewNoteUnitModel>? frontUnits;
  final List<String> context;
  final List<List<InlineRunModel>> contextRuns;
  final List<String> back;
  final List<List<InlineRunModel>> backRuns;
  final List<ReviewNoteUnitModel> backUnits;
  final bool reshaped;
  final List<ReviewNoteUnitModel> note;
  final List<ReviewImageModel> images;
  final List<ReviewImageModel> imagesBack;
}

class ReviewChoiceFeedbackModel {
  const ReviewChoiceFeedbackModel({
    required this.chosen,
    required this.correct,
    required this.passed,
  });

  final int chosen;
  final int correct;
  final bool passed;
}

class ReviewTypedResultModel {
  const ReviewTypedResultModel({
    required this.input,
    required this.expected,
    required this.passed,
  });

  final String input;
  final String expected;
  final bool passed;
}

class ReviewCheckFeedbackModel {
  ReviewCheckFeedbackModel({
    required Iterable<ReviewTypedResultModel> results,
    required this.passed,
  }) : results = List.unmodifiable(results);

  final List<ReviewTypedResultModel> results;
  final bool passed;
}

class ReviewForeignWriterModel {
  const ReviewForeignWriterModel({required this.device, required this.ageMs});

  final String device;
  final int ageMs;
}

class ReviewCrumbModel {
  ReviewCrumbModel({
    required Iterable<String> regions,
    required this.current,
    required Iterable<Iterable<String>> cells,
  }) : regions = List.unmodifiable(regions),
       cells = List.unmodifiable([
         for (final row in cells) List<String>.unmodifiable(row),
       ]);

  final List<String> regions;
  final int current;
  final List<List<String>> cells;
}

class ReviewTutorCardModel {
  ReviewTutorCardModel({
    required this.subject,
    required this.front,
    required Iterable<String> back,
    this.at,
    required this.line,
  }) : back = List.unmodifiable(back);

  final String subject;
  final String front;
  final List<String> back;
  final String? at;
  final int line;
}

class ReviewStateModel {
  ReviewStateModel({
    this.card,
    required this.mode,
    required this.depth,
    required this.acquire,
    Iterable<String>? choices,
    Iterable<Iterable<InlineRunModel>>? choiceRuns,
    Iterable<String>? keypoints,
    Iterable<Iterable<InlineRunModel>>? keypointRuns,
    required this.finished,
    required this.remaining,
    required this.reviews,
    required this.passed,
    required this.failed,
    required this.acquired,
    required this.recognized,
    required this.recognizePartly,
    required this.recognizeMissed,
    required this.canRestart,
    required this.promotable,
    this.nextDueMs,
    required this.dueLeft,
    required this.newLeft,
    this.saveError,
  }) : choices = choices == null ? null : List.unmodifiable(choices),
       choiceRuns = choiceRuns == null ? null : _freezeRunLines(choiceRuns),
       keypoints = keypoints == null ? null : List.unmodifiable(keypoints),
       keypointRuns = keypointRuns == null
           ? null
           : _freezeRunLines(keypointRuns);

  final ReviewCardModel? card;
  final ReviewMode mode;
  final ReviewDepth depth;
  final bool acquire;
  final List<String>? choices;
  final List<List<InlineRunModel>>? choiceRuns;
  final List<String>? keypoints;
  final List<List<InlineRunModel>>? keypointRuns;
  final bool finished;
  final int remaining;
  final int reviews;
  final int passed;
  final int failed;
  final int acquired;
  final int recognized;
  final int recognizePartly;
  final int recognizeMissed;
  final bool canRestart;
  final bool promotable;
  final int? nextDueMs;
  final int dueLeft;
  final int newLeft;
  final String? saveError;
}

List<List<InlineRunModel>> _freezeRunLines(
  Iterable<Iterable<InlineRunModel>> lines,
) {
  return List.unmodifiable([
    for (final line in lines) List<InlineRunModel>.unmodifiable(line),
  ]);
}
