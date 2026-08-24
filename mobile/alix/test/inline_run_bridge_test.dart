import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bridge/inline_run_bridge.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/src/rust/api/review.dart' as bridge;

void main() {
  test('the inline bridge maps every presentation field', () {
    const source = bridge.InlineRun(
      text: r'x^2',
      bold: true,
      italic: true,
      strike: true,
      code: false,
      link: true,
      sub: true,
      sup: true,
      ins: true,
      math: bridge.MathView(display: true, svg: '<svg/>', error: 'fallback'),
    );

    final model = inlineRunFromBridge(source);

    expect(
      model,
      const InlineRunModel(
        text: r'x^2',
        bold: true,
        italic: true,
        strike: true,
        code: false,
        link: true,
        sub: true,
        sup: true,
        ins: true,
        math: InlineMathModel(display: true, svg: '<svg/>', error: 'fallback'),
      ),
    );
  });

  test('the inline bridge preserves list shape and absent math', () {
    const plain = bridge.InlineRun(
      text: 'plain',
      bold: false,
      italic: false,
      strike: false,
      code: true,
      link: false,
      sub: false,
      sup: false,
      ins: false,
    );

    expect(inlineRunsFromBridge(const [plain]), const [
      InlineRunModel(text: 'plain', bold: false, italic: false, code: true),
    ]);
    expect(
      inlineRunLinesFromBridge(const [
        [plain],
        [],
      ]),
      const [
        [InlineRunModel(text: 'plain', bold: false, italic: false, code: true)],
        <InlineRunModel>[],
      ],
    );
  });
}
