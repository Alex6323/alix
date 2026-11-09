import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/picker_widgets.dart';

PickerEntry _entry({required bool examChanged}) => PickerEntry(
  title: 'rust',
  path: 'rust.md',
  isWorkspace: false,
  due: false,
  canRecognize: false,
  isTrace: false,
  mastered: false,
  passed: true,
  examBadge: 'passed (2 Nov) · last (4 Nov) 15 / 18',
  examChanged: examChanged,
  examDue: false,
  hasExam: true,
  locked: false,
  progressError: false,
  indent: 0,
  tree: '',
);

TextStyle? _badgeStyle(PickerEntry entry) {
  final line = pickerExamBadgeLine(ThemeData(), entry);
  expect(line, hasLength(1), reason: 'an examined deck has a badge line');
  return (line.single as Text).style;
}

void main() {
  test('an_exam_changed_badge_renders_dashed', () {
    final changed = _badgeStyle(_entry(examChanged: true));
    expect(changed?.decoration, TextDecoration.underline);
    expect(changed?.decorationStyle, TextDecorationStyle.dashed);

    final plain = _badgeStyle(_entry(examChanged: false));
    expect(
      plain?.decoration,
      isNull,
      reason: 'control: an unchanged badge keeps the plain text style',
    );
  });
}
