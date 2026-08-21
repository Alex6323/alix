import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

void main() {
  test('the phase-4 baseline mutation inventory is line-exact', () {
    expect(_linesContaining('lib/review_screen.dart', 'setState('), isEmpty);
    expect(
      _linesContaining(
        'lib/review/review_controller.dart',
        'notifyListeners();',
      ),
      [84, 89, 94, 99, 104, 109, 114, 119, 124, 129, 134, 139, 144, 149, 154, 159],
      reason:
          'setServerLive, install, choose, check, openAttempt, '
          'toggleKeypoint, reveal, the six sketch transitions '
          '(tool, begin, extend, end, undo, clear), revealNextLine, '
          'dismissForeignWriter, and restart own every ReviewController '
          'mutation',
    );
    expect(_linesContaining('lib/picker_screen.dart', 'setState('), isEmpty);
    expect(
      _linesContaining(
        'lib/picker/picker_controller.dart',
        'notifyListeners();',
      ),
      [48, 53, 69],
      reason:
          'setServerReachable, reload, and dismissConflicts own every picker '
          'listing mutation; deadline and tutorial transitions reload',
    );
    expect(
      _linesContaining(
        'lib/picker/generate_controller.dart',
        'notifyListeners();',
      ),
      [62, 111, 115, 123],
      reason:
          'begin, complete, progress, and fail own every generation mutation',
    );
    expect(_linesContaining('lib/walk_screen.dart', 'setState('), isEmpty);
    expect(
      _linesContaining('lib/walk/walk_controller.dart', 'notifyListeners();'),
      [43, 50, 55, 60],
      reason:
          'setServerLive, predict, grade, and restart are the four named '
          'WalkController mutations',
    );
    expect(
      [
        ..._sites('lib/review_screen.dart', 'ListenableBuilder('),
        ..._sites('lib/picker_screen.dart', 'ListenableBuilder('),
        ..._sites('lib/picker/generate_sheet.dart', 'ListenableBuilder('),
        ..._sites('lib/walk_screen.dart', 'ListenableBuilder('),
      ],
      [
        'lib/review_screen.dart:245',
        'lib/picker_screen.dart:311',
        'lib/picker/generate_sheet.dart:42',
        'lib/walk_screen.dart:192',
      ],
    );
  });

  test('the phase-4 baseline has one timer and no stream subscriptions', () {
    final paths = [
      'lib/review_screen.dart',
      'lib/picker_screen.dart',
      'lib/picker/generate_controller.dart',
      'lib/walk_screen.dart',
    ];
    expect(
      [for (final path in paths) ..._sites(path, 'Timer(')],
      ['lib/picker/generate_controller.dart:142'],
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
