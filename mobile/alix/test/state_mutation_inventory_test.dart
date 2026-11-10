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
        108,
        113,
        118,
        123,
        128,
        149,
        159,
        167,
        172,
        177,
        182,
        187,
        192,
        197,
        202,
        207,
        212,
        217,
        222,
        249,
      ],
      reason:
          'setServerLive, install, choose, toggleChoice, submitChoices, '
          'check, accepted first hint, next hint, openAttempt, toggleKeypoint, '
          'reveal, '
          'the six sketch transitions (tool, begin, extend, end, undo, '
          'clear), revealNextLine, restart, and applyCardNote own every '
          'ReviewController mutation',
    );
    expect(_linesContaining('lib/picker_screen.dart', 'setState('), isEmpty);
    expect(
      _linesContaining(
        'lib/picker/picker_controller.dart',
        'notifyListeners();',
      ),
      [86, 208],
      reason:
          'setServerReachable and _notifyIfLive own every picker mutation; '
          'listing, strip, and search transitions notify through it, and '
          'deadline and setPairedRoot transitions reload',
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
      [41, 46, 51],
      reason:
          'predict, grade, and restart are the three named '
          'TraceSessionController mutations',
    );
    expect(_linesContaining('lib/walk_screen.dart', 'setState('), isEmpty);
    expect(
      _linesContaining('lib/walk/walk_controller.dart', 'notifyListeners();'),
      [
        70, 77, 82, 89, 94, 104, 109, 119, 127,
        132, 137, 142, 147, 152, 157, 162, 177,
      ],
      reason:
          'setServerLive, choose, toggleChoice, submitChoices, reveal, '
          'revealNextLine, grade, openHint, openNextHint, nextWalk, the six '
          'sketch transitions, and applyCardNote own every WalkController '
          'mutation',
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
        'lib/review_screen.dart:391',
        'lib/picker_screen.dart:918',
        'lib/picker/generate_sheet.dart:42',
        'lib/trace_screen.dart:129',
        'lib/walk_screen.dart:222',
      ],
      reason:
          'sync wiring added imports, fields, and methods above build() in '
          'review_screen.dart and picker_screen.dart, the topic sheet '
          'opener became one shared Context/Hint panel in review_screen.dart, '
          'and the skip-introduction switch added a field to both, moving '
          'their single ListenableBuilder site; the tutor conversation '
          'ownership added an import, four fields, and three methods to '
          'review_screen.dart; the walk launch added an import and a method '
          'to picker_screen.dart; the session-end conflict check added '
          'imports, a field, and methods to trace_screen.dart; the shared '
          'showTopicSheet shortened the opener in review_screen.dart and '
          'walk_screen.dart by four lines; the review menu'"'"'s skip added a '
          'method to review_screen.dart; retirement moved into '
          'TutorConversation.retire, shortening both screens; the walk hint '
          'sheet added an import line and a method to walk_screen.dart; '
          'the tutorial entry added a constructor and the opener choice to '
          'walk_screen.dart and replaced the copy with a push in '
          'picker_screen.dart; prepared-exam polling and resume added fields '
          'and methods above picker_screen.dart build(), and the exam screen '
          'took over closing its client, removing a line above it; '
          'per-topic remediation passes the topic id to the bridge, adding '
          'a line above picker_screen.dart build(); '
          'the kept exam sitting start added a top-level slot and its '
          'reuse above picker_screen.dart build(); '
          'generate_sheet.dart is unchanged',
    );
  });

  test('the phase-4 baseline has two timers and no stream subscriptions', () {
    final paths = [
      'lib/review_screen.dart',
      'lib/picker_screen.dart',
      'lib/picker/generate_controller.dart',
      'lib/trace_screen.dart',
    ];
    expect(
      [
        for (final path in paths) ...[
          ..._sites(path, 'Timer('),
          ..._sites(path, 'Timer.periodic('),
        ],
      ],
      ['lib/picker_screen.dart:358', 'lib/picker/generate_controller.dart:142'],
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
