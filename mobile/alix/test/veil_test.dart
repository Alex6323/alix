// The hidden look of a revealed answer, the same law as the web's veilRuns:
// each word keeps its first character, and its last past two characters;
// the rest is a veiled run drawn transparent in place.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/shared/veil.dart';

String _shape(List<VeilRun> runs) =>
    runs.map((run) => run.kept ? run.text : '[${run.text}]').join();

void main() {
  test('the veil keeps a word\'s first letter, and its last past two letters', () {
    const rows = [
      ('one letter', 'a', 'a'),
      ('two letters', 'of', 'o[f]'),
      ('three letters', 'the', 't[h]e'),
      ('a long word', 'immutable', 'i[mmutabl]e'),
      ('attached punctuation counts as letters', 'binding,', 'b[inding],'),
      (
        'a code token is a word like any other',
        'let mut x = &mut x',
        'l[e]t m[u]t x = &[mu]t x',
      ),
      (
        'several words keep every space',
        'a name cannot be reassigned.',
        'a n[am]e c[anno]t b[e] r[eassigned].',
      ),
      ('leading and trailing space survive', '  two  ', '  t[w]o  '),
      ('a tab is whitespace too', 'x\ty', 'x\ty'),
      ('empty text is no runs', '', ''),
    ];
    for (final (label, text, expected) in rows) {
      expect(_shape(veilRuns(text)), expected, reason: label);
    }
  });

  test('the veiled runs concatenate back to the text, so the width is kept', () {
    for (final text in [
      '',
      'x',
      'an immutable binding: the name cannot be reassigned.',
      '`let mut x = &mut x`',
    ]) {
      expect(veilRuns(text).map((run) => run.text).join(), text, reason: text);
    }
  });

  test('runs alternate between kept and veiled, and no run is empty', () {
    final runs = veilRuns('hello wide world');
    expect(runs.every((run) => run.text.isNotEmpty), isTrue, reason: 'no empty run');
    for (var index = 1; index < runs.length; index++) {
      expect(runs[index].kept, isNot(runs[index - 1].kept), reason: 'run $index alternates');
    }
  });

  test('veiled spans keep the style and draw the veiled runs transparent', () {
    const style = TextStyle(color: Colors.white, fontSize: 18);
    final spans = veilSpans('immutable', style);
    expect(spans.map((span) => span.text).join(), 'immutable');
    expect(spans.map((span) => span.style?.color).toList(), [
      Colors.white,
      Colors.transparent,
      Colors.white,
    ]);
    expect(spans.every((span) => span.style?.fontSize == 18), isTrue);
  });
}
