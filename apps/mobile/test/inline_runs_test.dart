import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/inline_runs.dart';
import 'package:alix_mobile/src/rust/api/review.dart';

const _svg =
    '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="12" '
    'viewBox="0 0 40 12"><path d="M0 0h40v12H0z"/></svg>';
const _wideSvg =
    '<svg xmlns="http://www.w3.org/2000/svg" width="1000" height="80" '
    'viewBox="0 0 1000 80"><path d="M0 0h1000v80H0z"/></svg>';

InlineRun mathRun(
  String text, {
  bool display = false,
  String? svg = _svg,
  String? error,
}) {
  return InlineRun(
    text: text,
    bold: false,
    italic: false,
    code: false,
    math: MathView(display: display, svg: svg, error: error),
  );
}

InlineRun textRun(
  String text, {
  bool bold = false,
  bool italic = false,
  bool code = false,
}) {
  return InlineRun(text: text, bold: bold, italic: italic, code: code);
}

Widget testApp(
  List<InlineRun> runs, {
  Color foreground = const Color(0xFF1847A0),
  double width = 320,
  bool contextHoles = false,
}) {
  return MaterialApp(
    home: Scaffold(
      body: Center(
        child: SizedBox(
          width: width,
          child: InlineRuns(
            runs: runs,
            style: TextStyle(fontSize: 20, color: foreground),
            textAlign: TextAlign.center,
            contextHoles: contextHoles,
            holeColor: const Color(0xFF00AA44),
            mutedHoleColor: const Color(0xFF777777),
          ),
        ),
      ),
    ),
  );
}

Iterable<TextSpan> textSpans(InlineSpan span) sync* {
  if (span is TextSpan) {
    yield span;
    for (final child in span.children ?? const <InlineSpan>[]) {
      yield* textSpans(child);
    }
  }
}

void main() {
  testWidgets('inline math uses SVG, foreground color, and one label', (
    tester,
  ) async {
    const foreground = Color(0xFF1847A0);
    final semantics = tester.ensureSemantics();
    await tester.pumpWidget(
      testApp([
        textRun('Euler wrote '),
        mathRun(r'e^{i\pi} + 1 = 0'),
        textRun(' elegantly.', italic: true),
      ], foreground: foreground),
    );
    await tester.pumpAndSettle();

    expect(find.byType(SvgPicture), findsOneWidget);
    final picture = tester.widget<SvgPicture>(find.byType(SvgPicture));
    expect(
      picture.colorFilter,
      const ColorFilter.mode(foreground, BlendMode.srcIn),
    );

    expect(find.bySemanticsLabel(RegExp(r'e\^\{i\\pi\}')), findsOneWidget);
    semantics.dispose();
  });

  testWidgets('display math centers and scales within a phone width', (
    tester,
  ) async {
    await tester.pumpWidget(
      testApp([mathRun(r'\sum_{n=1}^{100} n^2', display: true, svg: _wideSvg)]),
    );
    await tester.pumpAndSettle();

    final fitted = find.byWidgetPredicate(
      (widget) => widget is FittedBox && widget.fit == BoxFit.scaleDown,
    );
    expect(fitted, findsOneWidget);
    expect(tester.getSize(fitted).width, lessThanOrEqualTo(320));
    expect(
      find.descendant(
        of: find.byType(InlineRuns),
        matching: find.byType(Center),
      ),
      findsOneWidget,
    );
    expect(tester.takeException(), isNull);
  });

  testWidgets('math errors and literal dollar code stay visible text', (
    tester,
  ) async {
    await tester.pumpWidget(
      testApp([
        mathRun(r'\frac{1', svg: null, error: 'unexpected end'),
        textRun(r' $5 and $10 '),
        textRun(r'$x$', code: true),
      ]),
    );

    expect(find.byType(SvgPicture), findsNothing);
    expect(
      find.textContaining('math could not render', findRichText: true),
      findsOneWidget,
    );
    expect(
      find.textContaining(r'$5 and $10', findRichText: true),
      findsOneWidget,
    );
    expect(find.textContaining(r'$x$', findRichText: true), findsOneWidget);
  });

  testWidgets('context holes style text while math remains intact', (
    tester,
  ) async {
    await tester.pumpWidget(
      testApp([
        textRun('the ____ term and […] sibling '),
        mathRun(r'x = \underline{\hspace{2em}} + \cdots'),
      ], contextHoles: true),
    );
    await tester.pumpAndSettle();

    final richText = tester.widget<RichText>(
      find.descendant(
        of: find.byType(InlineRuns),
        matching: find.byType(RichText),
      ),
    );
    final spans = textSpans(richText.text).toList();
    final active = spans.singleWhere((span) => span.text == '____');
    final hidden = spans.singleWhere((span) => span.text == '[…]');
    expect(active.style!.color, const Color(0xFF00AA44));
    expect(active.style!.fontWeight, FontWeight.w700);
    expect(hidden.style!.color, const Color(0xFF777777));
    expect(find.byType(SvgPicture), findsOneWidget);
  });
}
