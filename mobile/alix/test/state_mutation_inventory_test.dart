import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

void main() {
  test('the phase-4 baseline mutation inventory is line-exact', () {
    expect(_linesContaining('lib/review_screen.dart', 'setState('), [
      168,
      292,
      846,
      980,
      996,
      1071,
      1222,
      1244,
      1265,
      1277,
      1437,
      1538,
    ]);
    expect(_linesContaining('lib/picker_screen.dart', 'setState('), [
      144,
      158,
      276,
      294,
      381,
      402,
      808,
      849,
      1113,
      1231,
      1237,
      1251,
      1259,
      1274,
      1282,
      1290,
      1306,
    ]);
    expect(_linesContaining('lib/walk_screen.dart', 'setState('), [
      152,
      188,
      193,
      201,
    ]);
  });

  test('the phase-4 baseline has one timer and no stream subscriptions', () {
    final paths = [
      'lib/review_screen.dart',
      'lib/picker_screen.dart',
      'lib/walk_screen.dart',
    ];
    expect(
      [for (final path in paths) ..._sites(path, 'Timer(')],
      ['lib/picker_screen.dart:1307'],
    );
    expect([
      for (final path in paths) ..._sites(path, 'StreamSubscription'),
      for (final path in paths) ..._sites(path, '.listen('),
    ], isEmpty);
  });
}

List<int> _linesContaining(String path, String needle) {
  return [
    for (final (index, line) in File(path).readAsLinesSync().indexed)
      if (line.contains(needle)) index + 1,
  ];
}

List<String> _sites(String path, String needle) {
  return [for (final line in _linesContaining(path, needle)) '$path:$line'];
}
