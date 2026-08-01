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

  void predict(String text);

  WalkStateModel grade(WalkGrade grade);

  int? examCooldownMs(int nowMs);

  void applyExamPassed(int nowMs);

  void applyExamFailed(int nowMs);
}

class WalkOpenFailure implements Exception {
  const WalkOpenFailure(this.message);

  final String message;

  @override
  String toString() => message;
}
