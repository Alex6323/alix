import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/src/rust/api/review.dart' as bridge;

InlineRunModel inlineRunFromBridge(bridge.InlineRun run) {
  final math = run.math;
  return InlineRunModel(
    text: run.text,
    bold: run.bold,
    italic: run.italic,
    code: run.code,
    math: math == null
        ? null
        : InlineMathModel(
            display: math.display,
            svg: math.svg,
            error: math.error,
          ),
  );
}

List<InlineRunModel> inlineRunsFromBridge(Iterable<bridge.InlineRun> runs) {
  return [for (final run in runs) inlineRunFromBridge(run)];
}

List<List<InlineRunModel>> inlineRunLinesFromBridge(
  Iterable<Iterable<bridge.InlineRun>> lines,
) {
  return [for (final line in lines) inlineRunsFromBridge(line)];
}
