import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/shared/inline_models.dart';

enum WalkPhase { front, answer, done }

class WalkStateModel {
  WalkStateModel({
    required this.phase,
    this.card,
    required this.mode,
    this.input = ReviewInput.type,
    Iterable<String>? choices,
    Iterable<Iterable<InlineRunModel>>? choiceRuns,
    this.choicesMultiple,
    this.sectionFirst = false,
    required this.position,
    required this.total,
    this.label = '',
    this.saveError,
  }) : choices = choices == null ? null : List.unmodifiable(choices),
       choiceRuns = choiceRuns == null
           ? null
           : List.unmodifiable([
               for (final runs in choiceRuns)
                 List<InlineRunModel>.unmodifiable(runs),
             ]);

  final WalkPhase phase;
  final ReviewCardModel? card;
  final ReviewMode mode;
  final ReviewInput input;
  final List<String>? choices;
  final List<List<InlineRunModel>>? choiceRuns;
  final bool? choicesMultiple;
  final bool sectionFirst;
  final int position;
  final int total;
  final String label;
  final String? saveError;

  bool get done => phase == WalkPhase.done;

  int get left => total - position;

  /// A walk never grades or introduces, so depth and tallies are inert.
  ReviewStateModel get view => ReviewStateModel(
    card: card,
    mode: mode,
    depth: ReviewDepth.recall,
    input: input,
    introducing: false,
    sectionFirst: sectionFirst,
    choices: choices,
    choiceRuns: choiceRuns,
    choicesMultiple: choicesMultiple,
    finished: done,
    remaining: left,
    reviews: 0,
    passed: 0,
    failed: 0,
    introduced: 0,
    partial: 0,
    canRestart: false,
    dueLeft: 0,
    newLeft: 0,
    saveError: saveError,
  );
}
