import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void expectCountRule(WidgetTester tester, Finder title, String step) {
  final count = find.textContaining(RegExp(r'^\d+ left$'));
  final rule = find.byType(VerticalDivider);
  if (count.evaluate().isEmpty) {
    expect(rule, findsNothing, reason: '$step: no counter, no rule');
    return;
  }
  expect(rule, findsOneWidget, reason: '$step: one rule beside the counter');
  expect(
    tester.widget<VerticalDivider>(rule).thickness,
    0,
    reason: '$step: the rule is a device-pixel hairline',
  );
  final line = tester.getCenter(rule);
  final counter = tester.getRect(count);
  expect(
    (
      line.dx > tester.getRect(title).right,
      line.dx < counter.left,
      (line.dy - counter.center.dy).abs() < 0.5,
    ),
    (true, true, true),
    reason:
        '$step: rule at $line sits between the title and the counter '
        '$counter, level with it',
  );
}
