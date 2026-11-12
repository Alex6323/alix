
import 'package:flutter/material.dart';

import 'package:alix_mobile/theme.dart';

const _mono = 'IBM Plex Mono';

typedef SourceLineSpans = List<TextSpan>? Function(String text);

Widget sourceExcerptError(AlixTokens tokens, String message, {Key? key}) {
  return Container(
    key: key,
    width: double.infinity,
    padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
    decoration: BoxDecoration(
      border: Border.all(color: tokens.line),
      borderRadius: BorderRadius.circular(10),
    ),
    child: Text(
      message,
      style: TextStyle(
        color: tokens.dim,
        fontStyle: FontStyle.italic,
        fontSize: 13,
      ),
    ),
  );
}

Widget sourceExcerptBlock(
  AlixTokens tokens, {
  required String path,
  required List<(int, String)> lines,
  required bool truncated,
  SourceLineSpans? spansForLine,
}) {
  return Container(
    decoration: BoxDecoration(
      border: Border.all(color: tokens.line),
      borderRadius: BorderRadius.circular(10),
    ),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 7),
          child: Text(
            path,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              fontFamily: _mono,
              fontSize: 11,
              color: tokens.dim,
            ),
          ),
        ),
        Divider(height: 1, color: tokens.line),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
          child: SingleChildScrollView(
            key: const ValueKey('source-code'),
            scrollDirection: Axis.horizontal,
            child: _sourceLines(
              lines,
              truncated,
              tokens,
              TextStyle(fontFamily: _mono, fontSize: 13, color: tokens.text),
              spansForLine,
            ),
          ),
        ),
      ],
    ),
  );
}

Widget _sourceLines(
  List<(int, String)> lines,
  bool truncated,
  AlixTokens tokens,
  TextStyle codeStyle,
  SourceLineSpans? spansForLine,
) {
  return Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      for (final line in lines)
        _gutterLine(
          line.$1,
          line.$2,
          tokens,
          codeStyle,
          spansForLine?.call(line.$2),
        ),
      if (truncated)
        Padding(
          padding: const EdgeInsets.only(top: 4),
          child: Text(
            '… excerpt truncated',
            style: TextStyle(
              color: tokens.faint,
              fontSize: 11,
              fontStyle: FontStyle.italic,
            ),
          ),
        ),
    ],
  );
}

Widget _gutterLine(
  int number,
  String text,
  AlixTokens tokens,
  TextStyle style,
  List<TextSpan>? spans,
) {
  final body = spans == null
      ? Text(text, style: style, softWrap: false)
      : Text.rich(TextSpan(style: style, children: spans), softWrap: false);
  return Padding(
    padding: const EdgeInsets.symmetric(vertical: 1),
    child: Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SizedBox(
          width: 32,
          child: Text(
            '$number',
            style: TextStyle(
              fontFamily: _mono,
              fontSize: 12,
              color: tokens.faint,
            ),
          ),
        ),
        body,
      ],
    ),
  );
}
