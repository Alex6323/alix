import 'package:alix_mobile/review/review_models.dart';

abstract interface class ReviewPortFactory {
  ReviewPort open({
    required String deckPath,
    required String rootDir,
    ReviewDepth? depth,
    String? device,
  });
}

abstract interface class ReviewPort {
  ReviewStateModel get state;

  ReviewForeignWriterModel? get foreignWriter;

  ReviewTutorCardModel? get tutorCard;

  bool get deckHasExam;

  ReviewCrumbModel? crumb(int nowMs);

  ReviewChoiceFeedbackModel? choose(int chosen);

  ReviewCheckFeedbackModel? check(List<String> lines);

  ReviewStateModel introduce();

  ReviewStateModel grade(ReviewGrade grade);

  ReviewGrade keypointGrade({required int covered, required int total});

  String mintTutorCard({
    required String front,
    required List<String> back,
    required int nowMs,
  });

  void applyCardNote({required int line, required List<String> notes});

  void applyExamPassed(int nowMs);

  int applyRemediation({required String cardsText, required int nowMs});
}

class ReviewOpenFailure implements Exception {
  const ReviewOpenFailure(this.message);

  final String message;

  @override
  String toString() => message;
}
