import 'package:alix_mobile/shared/inline_models.dart';

enum ReviewDepth { recognize, recall, reconstruct }

enum ReviewMode { flip, typing, typeLine, choice, lineByLine, explain }

enum ReviewGrade { fail, partial, pass }

enum ReviewRegionRole { asked, mask, cover }

/// One drawable mask on an image (ADR 0034). Geometry is in the source
/// image's own pixels or percentages of the full source, per [unit].
class ReviewRegionModel {
  const ReviewRegionModel({
    required this.role,
    required this.revealOnAnswer,
    required this.x,
    required this.y,
    required this.width,
    required this.height,
    required this.unit,
  });

  final ReviewRegionRole role;

  /// Whether local answer reveal unmasks this region; never inferred from
  /// [role] (a cover reveals on an ordinary card, never on a region card).
  final bool revealOnAnswer;
  final double x;
  final double y;
  final double width;
  final double height;
  final String unit;
}

/// The visible viewport onto a source image; region coordinates stay in
/// full-source space.
class ReviewCropModel {
  const ReviewCropModel({
    required this.x,
    required this.y,
    required this.width,
    required this.height,
    required this.unit,
  });

  final double x;
  final double y;
  final double width;
  final double height;
  final String unit;
}

class ReviewImageModel {
  const ReviewImageModel({
    required this.src,
    this.alt,
    this.regions = const [],
    this.crop,
  });

  final String src;
  final String? alt;
  final List<ReviewRegionModel> regions;
  final ReviewCropModel? crop;
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

/// One post-answer note. [badge] is the GitHub alert badge that opened it,
/// null for a note no blockquote opened (a table's note column, an AI
/// augmentation, a personal note).
class ReviewNoteModel {
  ReviewNoteModel({this.badge, required Iterable<ReviewNoteUnitModel> units})
    : units = List.unmodifiable(units);

  final ReviewBadge? badge;
  final List<ReviewNoteUnitModel> units;
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

class ReviewDiagramModel extends ReviewNoteUnitModel {
  ReviewDiagramModel({
    required this.src,
    required this.width,
    required this.height,
    required this.alt,
    Iterable<ReviewRegionModel> regions = const [],
    this.revealedAlt,
  }) : regions = List.unmodifiable(regions);

  /// An absolute file path: the lean core serves diagrams from disk.
  final String src;

  /// Logical pixels; the raster behind [src] is 2x.
  final int width;
  final int height;
  final String alt;

  /// Overlay regions in RASTER pixel space (the PNG's own pixels); empty on
  /// an unmasked diagram.
  final List<ReviewRegionModel> regions;

  /// Post-answer accessible text: asked labels revealed, siblings and
  /// covers still masked. Null when nothing is masked.
  final String? revealedAlt;
}

class ReviewChecklistModel extends ReviewNoteUnitModel {
  ReviewChecklistModel(Iterable<ReviewChecklistItemModel> items)
    : items = List.unmodifiable(items);

  final List<ReviewChecklistItemModel> items;
}

enum ReviewCellAlign { none, left, center, right }

class ReviewTableModel extends ReviewNoteUnitModel {
  ReviewTableModel({
    required Iterable<ReviewCellAlign> aligns,
    required Iterable<Iterable<InlineRunModel>> header,
    required Iterable<Iterable<Iterable<InlineRunModel>>> rows,
  }) : aligns = List.unmodifiable(aligns),
       header = _freezeRunLines(header),
       rows = List.unmodifiable([for (final row in rows) _freezeRunLines(row)]);

  final List<ReviewCellAlign> aligns;
  final List<List<InlineRunModel>> header;

  /// Every row matches the header width: the lib pads and truncates.
  final List<List<List<InlineRunModel>>> rows;
}

class ReviewCardModel {
  ReviewCardModel({
    required this.front,
    required Iterable<InlineRunModel> frontRuns,
    Iterable<ReviewNoteUnitModel>? frontUnits,
    required Iterable<String> context,
    required this.contextLeads,
    required Iterable<Iterable<InlineRunModel>> contextRuns,
    required Iterable<ReviewNoteUnitModel> contextUnits,
    required Iterable<String> back,
    required Iterable<Iterable<InlineRunModel>> backRuns,
    required Iterable<ReviewNoteUnitModel> backUnits,
    required this.reshaped,
    required Iterable<ReviewNoteModel> note,
    required Iterable<ReviewImageModel> images,
    required Iterable<ReviewImageModel> imagesBack,
  }) : frontRuns = List.unmodifiable(frontRuns),
       frontUnits = frontUnits == null ? null : List.unmodifiable(frontUnits),
       context = List.unmodifiable(context),
       contextRuns = _freezeRunLines(contextRuns),
       contextUnits = List.unmodifiable(contextUnits),
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

  /// Whether `context` is the question (a cloze sentence) or a label for the
  /// front (a card table's title).
  final bool contextLeads;
  final List<List<InlineRunModel>> contextRuns;

  /// The context's raw fences and closed bare-math blocks, in source order.
  /// Context prose keeps its line rendering.
  final List<ReviewNoteUnitModel> contextUnits;
  final List<String> back;
  final List<List<InlineRunModel>> backRuns;
  final List<ReviewNoteUnitModel> backUnits;
  final bool reshaped;
  final List<ReviewNoteModel> note;
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

class ReviewMultiChoiceFeedbackModel {
  ReviewMultiChoiceFeedbackModel({
    required Iterable<int> chosen,
    required Iterable<int> correct,
    required this.passed,
  }) : chosen = Set.unmodifiable(chosen),
       correct = Set.unmodifiable(correct);

  final Set<int> chosen;
  final Set<int> correct;
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

/// How the learner answers: typed, or sketched on a canvas. Resolved in the
/// lib (`review::state`), never inferred here.
enum ReviewInput { type, draw }

/// GitHub's five alert badges, the closed set that opens a note.
enum ReviewBadge { note, tip, important, warning, caution }

class ReviewStateModel {
  ReviewStateModel({
    this.card,
    required this.mode,
    required this.depth,
    this.input = ReviewInput.type,
    required this.introducing,
    Iterable<String>? choices,
    Iterable<Iterable<InlineRunModel>>? choiceRuns,
    this.choicesMultiple,
    Iterable<String>? keypoints,
    Iterable<Iterable<InlineRunModel>>? keypointRuns,
    required this.finished,
    required this.remaining,
    required this.reviews,
    required this.passed,
    required this.failed,
    required this.introduced,
    required this.partial,
    required this.canRestart,
    this.nextDueMs,
    required this.dueLeft,
    required this.newLeft,
    this.saveError,
    Iterable<String> loadWarnings = const [],
  }) : choices = choices == null ? null : List.unmodifiable(choices),
       choiceRuns = choiceRuns == null ? null : _freezeRunLines(choiceRuns),
       keypoints = keypoints == null ? null : List.unmodifiable(keypoints),
       keypointRuns = keypointRuns == null
           ? null
           : _freezeRunLines(keypointRuns),
       loadWarnings = List.unmodifiable(loadWarnings);

  final ReviewCardModel? card;
  final ReviewMode mode;
  final ReviewDepth depth;
  final ReviewInput input;
  final bool introducing;
  final List<String>? choices;
  final List<List<InlineRunModel>>? choiceRuns;
  final bool? choicesMultiple;
  final List<String>? keypoints;
  final List<List<InlineRunModel>>? keypointRuns;
  final bool finished;
  final int remaining;
  final int reviews;
  final int passed;
  final int failed;
  final int introduced;
  final int partial;
  final bool canRestart;
  final int? nextDueMs;
  final int dueLeft;
  final int newLeft;
  final String? saveError;
  final List<String> loadWarnings;
}

List<List<InlineRunModel>> _freezeRunLines(
  Iterable<Iterable<InlineRunModel>> lines,
) {
  return List.unmodifiable([
    for (final line in lines) List<InlineRunModel>.unmodifiable(line),
  ]);
}
