import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';

import 'package:alix_mobile/src/rust/api/review.dart';

const _mono = 'IBM Plex Mono';

class InlineRuns extends StatelessWidget {
  const InlineRuns({
    super.key,
    required this.runs,
    required this.style,
    this.textAlign = TextAlign.start,
    this.contextHoles = false,
    this.holeColor,
    this.mutedHoleColor,
  });

  final List<InlineRun> runs;
  final TextStyle style;
  final TextAlign textAlign;
  final bool contextHoles;
  final Color? holeColor;
  final Color? mutedHoleColor;

  @override
  Widget build(BuildContext context) {
    final blocks = <Widget>[];
    var inline = <InlineRun>[];

    void flushInline() {
      if (inline.isEmpty) return;
      blocks.add(_inlineText(inline));
      inline = <InlineRun>[];
    }

    for (final run in runs) {
      if (run.math?.display ?? false) {
        flushInline();
        blocks.add(_displayMath(run));
      } else {
        inline.add(run);
      }
    }
    flushInline();

    if (blocks.isEmpty) return const SizedBox.shrink();
    if (blocks.length == 1) return blocks.single;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: _crossAxisAlignment(textAlign),
      children: [
        for (final (index, block) in blocks.indexed) ...[
          if (index > 0) const SizedBox(height: 8),
          block,
        ],
      ],
    );
  }

  Widget _inlineText(List<InlineRun> inline) {
    final standaloneMath = inline.length == 1 && inline.single.math != null;
    return Text.rich(
      TextSpan(
        style: style,
        children: [
          for (final run in inline)
            ..._inlineSpans(run, standaloneMath: standaloneMath),
        ],
      ),
      textAlign: textAlign,
      softWrap: true,
    );
  }

  List<InlineSpan> _inlineSpans(InlineRun run, {required bool standaloneMath}) {
    final math = run.math;
    if (math != null) {
      final svg = math.svg;
      if (svg != null) {
        return [
          WidgetSpan(
            alignment: PlaceholderAlignment.baseline,
            baseline: TextBaseline.alphabetic,
            child: Semantics(
              label: run.text,
              child: ExcludeSemantics(
                child: SvgPicture.string(
                  svg,
                  height:
                      (style.fontSize ?? 14) * (standaloneMath ? 1.45 : 1.2),
                  fit: BoxFit.contain,
                  colorFilter: ColorFilter.mode(
                    style.color ?? Colors.black,
                    BlendMode.srcIn,
                  ),
                ),
              ),
            ),
          ),
        ];
      }
      return _mathErrorSpans(run);
    }
    if (!contextHoles) {
      return [TextSpan(text: run.text, style: _runStyle(run))];
    }
    return _contextSpans(run);
  }

  List<InlineSpan> _contextSpans(InlineRun run) {
    final spans = <InlineSpan>[];
    final marker = RegExp(r'____|\[…]');
    var start = 0;
    for (final match in marker.allMatches(run.text)) {
      if (match.start > start) {
        spans.add(
          TextSpan(
            text: run.text.substring(start, match.start),
            style: _runStyle(run),
          ),
        );
      }
      final text = match.group(0) ?? '';
      spans.add(
        TextSpan(
          text: text,
          style: _runStyle(run).copyWith(
            color: text == '____' ? holeColor : mutedHoleColor,
            fontWeight: text == '____'
                ? FontWeight.w700
                : _runStyle(run).fontWeight,
          ),
        ),
      );
      start = match.end;
    }
    if (start < run.text.length) {
      spans.add(
        TextSpan(text: run.text.substring(start), style: _runStyle(run)),
      );
    }
    return spans;
  }

  List<InlineSpan> _mathErrorSpans(InlineRun run) {
    return [
      TextSpan(
        text: run.text,
        style: style.copyWith(fontFamily: _mono),
      ),
      TextSpan(
        text: '  math could not render',
        style: style.copyWith(
          fontFamily: _mono,
          fontStyle: FontStyle.italic,
          color: (style.color ?? Colors.black).withValues(alpha: 0.72),
        ),
      ),
    ];
  }

  Widget _displayMath(InlineRun run) {
    final svg = run.math?.svg;
    if (svg == null) {
      return Text.rich(
        TextSpan(style: style, children: _mathErrorSpans(run)),
        textAlign: textAlign,
      );
    }
    return Semantics(
      label: run.text,
      child: ExcludeSemantics(
        child: LayoutBuilder(
          builder: (context, constraints) {
            return Center(
              child: ConstrainedBox(
                constraints: BoxConstraints(maxWidth: constraints.maxWidth),
                child: FittedBox(
                  fit: BoxFit.scaleDown,
                  child: SvgPicture.string(
                    svg,
                    colorFilter: ColorFilter.mode(
                      style.color ?? Colors.black,
                      BlendMode.srcIn,
                    ),
                  ),
                ),
              ),
            );
          },
        ),
      ),
    );
  }

  TextStyle _runStyle(InlineRun run) {
    return style.copyWith(
      fontFamily: run.code ? _mono : style.fontFamily,
      fontWeight: run.bold ? FontWeight.w700 : style.fontWeight,
      fontStyle: run.italic ? FontStyle.italic : style.fontStyle,
    );
  }
}

CrossAxisAlignment _crossAxisAlignment(TextAlign align) {
  return switch (align) {
    TextAlign.center => CrossAxisAlignment.center,
    TextAlign.right || TextAlign.end => CrossAxisAlignment.end,
    TextAlign.justify => CrossAxisAlignment.stretch,
    TextAlign.left || TextAlign.start => CrossAxisAlignment.start,
  };
}
