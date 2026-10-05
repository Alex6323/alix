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
      [
        107,
        112,
        117,
        122,
        127,
        148,
        153,
        158,
        163,
        168,
        173,
        178,
        183,
        188,
        193,
        198,
        203,
        224,
      ],
      reason:
          'setServerLive, install, choose, toggleChoice, submitChoices, '
          'check, openAttempt, toggleKeypoint, reveal, the six sketch '
          'transitions (tool, begin, extend, end, undo, clear), '
          'revealNextLine, restart, and applyCardNote own every '
          'ReviewController mutation',
    );
    expect(_linesContaining('lib/picker_screen.dart', 'setState('), isEmpty);
    expect(
      _linesContaining(
        'lib/picker/picker_controller.dart',
        'notifyListeners();',
      ),
      [83, 163],
      reason:
          'setServerReachable and _notifyIfLive own every picker mutation; '
          'listing, strip, and search transitions notify through it, and '
          'deadline, tutorial, and setPairedRoot transitions reload',
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
    expect(_linesContaining('lib/trace_screen.dart', 'setState('), isEmpty);
    expect(
      _linesContaining('lib/trace/trace_controller.dart', 'notifyListeners();'),
      [43, 50, 55, 60],
      reason:
          'setServerLive, predict, grade, and restart are the four named '
          'TraceSessionController mutations',
    );
    expect(_linesContaining('lib/walk_screen.dart', 'setState('), isEmpty);
    expect(
      _linesContaining('lib/walk/walk_controller.dart', 'notifyListeners();'),
      [56, 63, 68, 75, 80, 85, 90, 95, 100, 105, 110, 115, 120, 131],
      reason:
          'setServerLive, choose, toggleChoice, submitChoices, reveal, next, '
          'nextWalk, the six sketch transitions, and applyCardNote own every '
          'WalkController mutation',
    );
    expect(
      [
        ..._sites('lib/review_screen.dart', 'ListenableBuilder('),
        ..._sites('lib/picker_screen.dart', 'ListenableBuilder('),
        ..._sites('lib/picker/generate_sheet.dart', 'ListenableBuilder('),
        ..._sites('lib/trace_screen.dart', 'ListenableBuilder('),
        ..._sites('lib/walk_screen.dart', 'ListenableBuilder('),
      ],
      [
        'lib/review_screen.dart:373',
        'lib/picker_screen.dart:648',
        'lib/picker/generate_sheet.dart:42',
        'lib/trace_screen.dart:192',
        'lib/walk_screen.dart:202',
      ],
      reason:
          'sync wiring added imports, fields, and methods above build() in '
          'review_screen.dart and picker_screen.dart, the section sheet '
          'opener added one import and one method to review_screen.dart, '
          'and the skip-introduction switch added a field to both, moving '
          'their single ListenableBuilder site; the tutor conversation '
          'ownership added an import, four fields, and three methods to '
          'review_screen.dart; the walk launch added an import and a method '
          'to picker_screen.dart; generate_sheet.dart and trace_screen.dart '
          'are unchanged',
    );
  });

  test('the phase-4 baseline has one timer and no stream subscriptions', () {
    final paths = [
      'lib/review_screen.dart',
      'lib/picker_screen.dart',
      'lib/picker/generate_controller.dart',
      'lib/trace_screen.dart',
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
