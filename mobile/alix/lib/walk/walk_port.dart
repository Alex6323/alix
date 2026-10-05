import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/walk/walk_models.dart';

abstract interface class WalkPortFactory {
  WalkPort open({
    required String deckPath,
    required String rootDir,
    String? device,
  });
}

abstract interface class WalkPort {
  WalkStateModel get state;

  ReviewTutorCardModel? get tutorCard;

  ReviewChoiceFeedbackModel? choose(int chosen);

  ReviewMultiChoiceFeedbackModel? chooseMulti(List<int> chosen);

  WalkStateModel reveal();

  WalkStateModel next();

  WalkStateModel restart();

  String mintTutorCard({required String front, required List<String> back});

  void applyCardNote({required String id, required List<String> notes});
}

class WalkOpenFailure implements Exception {
  const WalkOpenFailure(this.message);

  final String message;

  @override
  String toString() => message;
}
