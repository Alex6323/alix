import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/shared/blank_pill.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/review/unit_widgets.dart';
import 'package:alix_mobile/shared/inline_runs.dart';
import 'package:alix_mobile/theme.dart';

const _svg =
    '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="12" '
    'viewBox="0 0 40 12"><path d="M0 0h40v12H0z"/></svg>';
const _wideSvg =
    '<svg xmlns="http://www.w3.org/2000/svg" width="1000" height="80" '
    'viewBox="0 0 1000 80"><path d="M0 0h1000v80H0z"/></svg>';

const _blank = Color(0xFF00AA44);
const _mutedBlank = Color(0x77777777);

BorderSide pillSide(WidgetTester tester, Finder pill) {
  final box = tester.widget<DecoratedBox>(
    find.descendant(of: pill, matching: find.byType(DecoratedBox)),
  );
  return ((box.decoration as ShapeDecoration).shape as StadiumBorder).side;
}

InlineRunModel mathRun(
  String text, {
  bool display = false,
  String? svg = _svg,
  String? error,
}) {
  return InlineRunModel(
    text: text,
    bold: false,
    italic: false,
    code: false,
    math: InlineMathModel(display: display, svg: svg, error: error),
  );
}

InlineRunModel textRun(
  String text, {
  bool bold = false,
  bool italic = false,
  bool code = false,
}) {
  return InlineRunModel(text: text, bold: bold, italic: italic, code: code);
}

Widget testApp(
  List<InlineRunModel> runs, {
  Color foreground = const Color(0xFF1847A0),
  double width = 320,
  bool contextHoles = false,
  double textScale = 1,
}) {
  return MaterialApp(
    builder: (context, child) => MediaQuery(
      data: MediaQuery.of(
        context,
      ).copyWith(textScaler: TextScaler.linear(textScale)),
      child: child!,
    ),
    home: Scaffold(
      body: Center(
        child: SizedBox(
          width: width,
          child: InlineRuns(
            runs: runs,
            style: TextStyle(fontSize: 20, color: foreground),
            textAlign: TextAlign.center,
            contextHoles: contextHoles,
            blankColor: _blank,
            mutedBlankColor: _mutedBlank,
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
  testWidgets('subscript and superscript use opposite baselines', (
    tester,
  ) async {
    await tester.pumpWidget(
      testApp(const [
        InlineRunModel(text: 'A', bold: false, italic: false, code: false),
        InlineRunModel(
          text: '2',
          bold: false,
          italic: false,
          code: false,
          sub: true,
        ),
        InlineRunModel(text: ' ', bold: false, italic: false, code: false),
        InlineRunModel(
          text: '3',
          bold: false,
          italic: false,
          code: false,
          sup: true,
        ),
      ]),
    );

    final paragraph = tester.renderObject<RenderParagraph>(
      find
          .descendant(
            of: find.byType(InlineRuns),
            matching: find.byType(RichText),
          )
          .first,
    );
    final sub = paragraph
        .getBoxesForSelection(
          const TextSelection(baseOffset: 1, extentOffset: 2),
        )
        .single;
    final sup = paragraph
        .getBoxesForSelection(
          const TextSelection(baseOffset: 3, extentOffset: 4),
        )
        .single;

    expect(sub.top, greaterThan(sup.top));
  });

  testWidgets('a blank in a script run sits on its baseline at its size', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    await tester.pumpWidget(
      testApp(const [
        InlineRunModel(text: 'A', bold: false, italic: false, code: false),
        InlineRunModel(
          text: '⍰',
          bold: false,
          italic: false,
          code: false,
          sub: true,
        ),
        InlineRunModel(text: ' ', bold: false, italic: false, code: false),
        InlineRunModel(
          text: '⬚',
          bold: false,
          italic: false,
          code: false,
          sup: true,
        ),
      ], contextHoles: true),
    );

    final sub = tester.getRect(find.bySemanticsLabel('blank'));
    final sup = tester.getRect(find.bySemanticsLabel('hidden blank'));
    expect(sub.top, greaterThan(sup.top), reason: 'sub $sub, sup $sup');
    expect(sub.width, 5 * 20 * 0.75, reason: 'sub pill at script size');
    expect(sup.width, sub.width, reason: 'asked and hidden widths match');
    semantics.dispose();
  });

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

  testWidgets('standalone inline math is larger than math inside prose', (
    tester,
  ) async {
    await tester.pumpWidget(testApp([mathRun(r'x^2 + y^2 = 1')]));
    await tester.pumpAndSettle();
    final standalone = tester.widget<SvgPicture>(find.byType(SvgPicture));

    await tester.pumpWidget(
      testApp([textRun('Use '), mathRun(r'x^2 + y^2 = 1'), textRun('.')]),
    );
    await tester.pumpAndSettle();
    final inProse = tester.widget<SvgPicture>(find.byType(SvgPicture));

    expect(standalone.height, 29);
    expect(inProse.height, 24);
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

  testWidgets('blanks draw as pills while math remains intact', (tester) async {
    await tester.pumpWidget(
      testApp([
        textRun('the ⍰ term and ⬚ sibling '),
        mathRun(r'x = \underline{\hspace{2em}} + \cdots'),
      ], contextHoles: true),
    );
    await tester.pumpAndSettle();

    expect(find.byType(BlankPill), findsNWidgets(2));
    expect(find.byType(SvgPicture), findsOneWidget);
  });

  testWidgets('every blank is an empty fixed-width outlined pill', (
    tester,
  ) async {
    final semanticsHandle = tester.ensureSemantics();
    final styles = <String, InlineRunModel Function(String)>{
      'plain': (text) => textRun(text),
      'bold': (text) => textRun(text, bold: true),
      'italic': (text) => textRun(text, italic: true),
      'code': (text) => textRun(text, code: true),
    };
    for (final MapEntry(key: label, value: run) in styles.entries) {
      for (final (marker, semantics, color) in [
        ('⍰', 'blank', _blank),
        ('⬚', 'hidden blank', _mutedBlank),
      ]) {
        final step = '$label $marker';
        await tester.pumpWidget(
          testApp([run('a $marker b')], contextHoles: true),
        );
        final pill = find.byType(BlankPill);
        expect(pill, findsOneWidget, reason: step);
        expect(tester.getSize(pill), const Size(100, 24), reason: step);
        final side = pillSide(tester, pill);
        expect((side.color, side.width), (color, 2.0), reason: step);
        expect(find.bySemanticsLabel(semantics), findsOneWidget, reason: step);
        final glyphs = tester
            .widgetList<RichText>(find.byType(RichText))
            .expand((rich) => textSpans(rich.text))
            .where((span) => (span.text ?? '').contains(RegExp('⍰|⬚')));
        expect(glyphs, isEmpty, reason: '$step: no glyph is drawn');
      }
    }
    semanticsHandle.dispose();
  });

  testWidgets(
    'both pill outlines clear 3:1 against the card in light and dark',
    (tester) async {
      double luminance(Color color) => color.computeLuminance();
      double contrast(Color a, Color b) {
        final (hi, lo) = luminance(a) > luminance(b)
            ? (luminance(a), luminance(b))
            : (luminance(b), luminance(a));
        return (hi + 0.05) / (lo + 0.05);
      }

      for (final (name, theme) in [
        ('light', alixLight()),
        ('dark', alixDark()),
      ]) {
        await tester.pumpWidget(
          MaterialApp(
            theme: theme,
            home: Builder(
              builder: (context) => Scaffold(
                body: runsOrText(
                  [textRun('a ⍰ b ⬚ c')],
                  'a ⍰ b ⬚ c',
                  style: const TextStyle(fontSize: 20),
                  contextHoles: true,
                  tokens: Theme.of(context).alix,
                ),
              ),
            ),
          ),
        );
        final card = theme.scaffoldBackgroundColor;
        for (final pill in tester.widgetList<BlankPill>(
          find.byType(BlankPill),
        )) {
          final outline = Color.alphaBlend(pill.color, card);
          final ratio = contrast(outline, card);
          expect(
            ratio,
            greaterThanOrEqualTo(3.0),
            reason:
                '$name ${pill.asked ? 'asked' : 'hidden'} pill: '
                '${ratio.toStringAsFixed(2)}:1',
          );
        }
      }
    },
  );

  testWidgets('a pill grows with the text scale', (tester) async {
    await tester.pumpWidget(
      testApp([textRun('a ⍰ b')], contextHoles: true, textScale: 1.5),
    );
    expect(tester.getSize(find.byType(BlankPill)).width, 150);
  });

  testWidgets('a filled answer takes the blank colour', (tester) async {
    await tester.pumpWidget(
      testApp([
        textRun('The '),
        const InlineRunModel(
          text: 'scaphoid',
          bold: false,
          italic: false,
          code: false,
          answer: true,
        ),
        textRun(' bone'),
      ], contextHoles: true),
    );
    final spans = tester
        .widgetList<RichText>(find.byType(RichText))
        .expand((rich) => textSpans(rich.text))
        .toList();
    expect(spans.singleWhere((s) => s.text == 'scaphoid').style?.color, _blank);
    expect(
      spans.singleWhere((s) => s.text == ' bone').style?.color,
      const Color(0xFF1847A0),
    );
  });

  testWidgets('a code run takes the theme code ink under a mono base style', (
    tester,
  ) async {
    const base = Color(0xFFEEEEEE);
    for (final theme in alixThemes) {
      await tester.pumpWidget(
        MaterialApp(
          theme: theme.data,
          home: Scaffold(
            body: Center(
              child: SizedBox(
                width: 320,
                child: InlineRuns(
                  runs: [
                    textRun('it trusts '),
                    textRun('block_fingerprint', code: true),
                  ],
                  style: const TextStyle(
                    fontFamily: 'IBM Plex Mono',
                    fontSize: 18,
                    color: base,
                  ),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      final spans = tester
          .widgetList<RichText>(
            find.descendant(
              of: find.byType(InlineRuns),
              matching: find.byType(RichText),
            ),
          )
          .expand((rich) => textSpans(rich.text))
          .toList();
      final code = spans.singleWhere(
        (span) => span.text == 'block_fingerprint',
      );
      final prose = spans.singleWhere((span) => span.text == 'it trusts ');

      expect(
        code.style?.color,
        theme.data.alix.code,
        reason: '${theme.id}: a code run takes --code',
      );
      expect(
        prose.style?.color,
        base,
        reason: '${theme.id}: prose keeps the caller\'s ink',
      );
      expect(
        code.style?.color,
        isNot(prose.style?.color),
        reason: '${theme.id}: a code run may not vanish into mono prose',
      );
    }
  });
}
