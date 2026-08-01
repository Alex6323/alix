import 'package:alix_mobile/shared/inline_models.dart';

enum WalkPhaseModel { predict, reveal, done }

enum WalkGrade { missed, partly, got }

class WalkLineModel {
  const WalkLineModel({required this.number, required this.text});

  final int number;
  final String text;
}

class WalkExcerptModel {
  WalkExcerptModel({
    required this.path,
    required Iterable<WalkLineModel> lines,
    required this.truncated,
  }) : lines = List.unmodifiable(lines);

  final String path;
  final List<WalkLineModel> lines;
  final bool truncated;
}

class WalkSummaryModel {
  WalkSummaryModel({
    required this.passed,
    required this.partly,
    required this.failed,
    required Iterable<int> weak,
    required this.total,
  }) : weak = List.unmodifiable(weak);

  final int passed;
  final int partly;
  final int failed;
  final List<int> weak;
  final int total;
}

class WalkStateModel {
  WalkStateModel({
    required this.phase,
    required this.description,
    required Iterable<InlineRunModel> descriptionRuns,
    this.source,
    required this.total,
    required this.current,
    this.prompt,
    Iterable<InlineRunModel>? promptRuns,
    required Iterable<String> givens,
    required Iterable<Iterable<InlineRunModel>> givenRuns,
    this.locator,
    this.prediction,
    this.excerpt,
    this.excerptError,
    required Iterable<String> points,
    required Iterable<Iterable<InlineRunModel>> pointRuns,
    this.note,
    Iterable<InlineRunModel>? noteRuns,
    this.summary,
    this.saveError,
  }) : descriptionRuns = List.unmodifiable(descriptionRuns),
       promptRuns = promptRuns == null ? null : List.unmodifiable(promptRuns),
       givens = List.unmodifiable(givens),
       givenRuns = _freezeNested(givenRuns),
       points = List.unmodifiable(points),
       pointRuns = _freezeNested(pointRuns),
       noteRuns = noteRuns == null ? null : List.unmodifiable(noteRuns);

  final WalkPhaseModel phase;
  final String description;
  final List<InlineRunModel> descriptionRuns;
  final String? source;
  final int total;
  final int current;
  final String? prompt;
  final List<InlineRunModel>? promptRuns;
  final List<String> givens;
  final List<List<InlineRunModel>> givenRuns;
  final String? locator;
  final String? prediction;
  final WalkExcerptModel? excerpt;
  final String? excerptError;
  final List<String> points;
  final List<List<InlineRunModel>> pointRuns;
  final String? note;
  final List<InlineRunModel>? noteRuns;
  final WalkSummaryModel? summary;
  final String? saveError;

  static const _unchanged = Object();

  WalkStateModel copyWith({
    WalkPhaseModel? phase,
    String? description,
    Iterable<InlineRunModel>? descriptionRuns,
    Object? source = _unchanged,
    int? total,
    int? current,
    Object? prompt = _unchanged,
    Object? promptRuns = _unchanged,
    Iterable<String>? givens,
    Iterable<Iterable<InlineRunModel>>? givenRuns,
    Object? locator = _unchanged,
    Object? prediction = _unchanged,
    Object? excerpt = _unchanged,
    Object? excerptError = _unchanged,
    Iterable<String>? points,
    Iterable<Iterable<InlineRunModel>>? pointRuns,
    Object? note = _unchanged,
    Object? noteRuns = _unchanged,
    Object? summary = _unchanged,
    Object? saveError = _unchanged,
  }) {
    return WalkStateModel(
      phase: phase ?? this.phase,
      description: description ?? this.description,
      descriptionRuns: descriptionRuns ?? this.descriptionRuns,
      source: identical(source, _unchanged) ? this.source : source as String?,
      total: total ?? this.total,
      current: current ?? this.current,
      prompt: identical(prompt, _unchanged) ? this.prompt : prompt as String?,
      promptRuns: identical(promptRuns, _unchanged)
          ? this.promptRuns
          : promptRuns as Iterable<InlineRunModel>?,
      givens: givens ?? this.givens,
      givenRuns: givenRuns ?? this.givenRuns,
      locator: identical(locator, _unchanged)
          ? this.locator
          : locator as String?,
      prediction: identical(prediction, _unchanged)
          ? this.prediction
          : prediction as String?,
      excerpt: identical(excerpt, _unchanged)
          ? this.excerpt
          : excerpt as WalkExcerptModel?,
      excerptError: identical(excerptError, _unchanged)
          ? this.excerptError
          : excerptError as String?,
      points: points ?? this.points,
      pointRuns: pointRuns ?? this.pointRuns,
      note: identical(note, _unchanged) ? this.note : note as String?,
      noteRuns: identical(noteRuns, _unchanged)
          ? this.noteRuns
          : noteRuns as Iterable<InlineRunModel>?,
      summary: identical(summary, _unchanged)
          ? this.summary
          : summary as WalkSummaryModel?,
      saveError: identical(saveError, _unchanged)
          ? this.saveError
          : saveError as String?,
    );
  }
}

List<List<InlineRunModel>> _freezeNested(
  Iterable<Iterable<InlineRunModel>> lines,
) {
  return List.unmodifiable([
    for (final line in lines) List<InlineRunModel>.unmodifiable(line),
  ]);
}
