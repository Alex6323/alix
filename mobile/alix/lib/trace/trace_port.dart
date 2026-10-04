import 'package:alix_mobile/trace/trace_models.dart';

abstract interface class TraceSessionPortFactory {
  TraceSessionPort open({
    required String deckPath,
    required String rootDir,
    String? device,
  });
}

abstract interface class TraceSessionPort {
  TraceSessionStateModel get state;

  void predict(String text);

  TraceSessionStateModel grade(TraceSessionGrade grade);

  int? examCooldownMs(int nowMs);

  void applyExamPassed(int nowMs);

  void applyExamFailed(int nowMs);
}

class TraceSessionOpenFailure implements Exception {
  const TraceSessionOpenFailure(this.message);

  final String message;

  @override
  String toString() => message;
}
