// Regression coverage for trace routing and acquire-only review summaries.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/walk_screen.dart';

import 'support/deck_fixture.dart';

const traceDeck = '''
---
trace: Parser walk
source: assets/tracemd/sha256-8093e53dbc3fe8722bd6cc3701e91bc9226f5b2529035008ec4339d09339ff9e.rs + sha256-f918dcc184b484285f8aa0494af33fb197a75902be5f781247074857038ae584.rs
---
## A `ReadingCloze` block just ended. What is handed to `parse_cloze_cards`?
The accumulated cloze `text` plus the block's start and end line numbers.
<!-- at: sha256-8093e53dbc3fe8722bd6cc3701e91bc9226f5b2529035008ec4339d09339ff9e.rs:1 from src/parser.rs:406-447 -->

## Why does this scan use bytes rather than chars?
Cloze `start`/`end` are byte positions, so counting must be in bytes.
<!-- at: sha256-f918dcc184b484285f8aa0494af33fb197a75902be5f781247074857038ae584.rs:1 from src/parser.rs:470-529 -->
''';

Directory traceRoot() {
  final root = Directory.systemTemp.createTempSync('alix-trace-routing-');
  final assets = Directory('${root.path}/assets/tracemd')
    ..createSync(recursive: true);
  File(
    '${assets.path}/sha256-8093e53dbc3fe8722bd6cc3701e91bc9226f5b2529035008ec4339d09339ff9e.rs',
  ).writeAsStringSync('parse_cloze_cards evidence\n');
  File(
    '${assets.path}/sha256-f918dcc184b484285f8aa0494af33fb197a75902be5f781247074857038ae584.rs',
  ).writeAsStringSync('byte offsets evidence\n');
  writeTestDeck('${root.path}/trace.md', traceDeck);
  writeTestDeck('${root.path}/facts.md', '# Facts\n\n## q?\na\n');
  return root;
}

void main() {
  setUpAll(() async => RustLib.init());

  testWidgets(
    'the picker marks a trace deck and opens it as an on-device walk, never a review',
    (tester) async {
      final root = traceRoot();
      addTearDown(() => root.deleteSync(recursive: true));
      final support = Directory.systemTemp.createTempSync(
        'alix-trace-routing-support-',
      );
      addTearDown(() => support.deleteSync(recursive: true));

      await tester.pumpWidget(
        MaterialApp(
          theme: alixDark(),
          home: PickerScreen(root: root.path, supportDir: support),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Parser walk'), findsOneWidget);
      expect(find.text('trace'), findsOneWidget, reason: 'the row is marked');

      await tester.tap(find.text('Parser walk'));
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
      expect(
        find.byType(ReviewScreen),
        findsNothing,
        reason: 'a trace deck must not open a review session',
      );
      expect(
        find.byType(WalkScreen),
        findsOneWidget,
        reason: 'it walks instead of refusing',
      );
    },
  );

  testWidgets('an acquire-only first pass says new cards were met, not zeros', (
    tester,
  ) async {
    final root = traceRoot();
    addTearDown(() => root.deleteSync(recursive: true));

    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: ReviewScreen(
          deckPath: '${root.path}/facts.md',
          rootDir: root.path,
          depth: Depth.recall,
        ),
      ),
    );
    await tester.pumpAndSettle();

    // The fresh deck's single card is acquired (Reveal, then Seen).
    await tester.tap(find.text('Reveal'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Seen'));
    await tester.pumpAndSettle();

    expect(find.text('SESSION COMPLETE'), findsOneWidget);
    expect(
      find.text('New cards planted.'),
      findsOneWidget,
      reason: 'an acquire-only sitting is not "Nothing due."',
    );
    expect(find.text('introduced'), findsOneWidget);
    expect(
      find.text('passed / failed'),
      findsNothing,
      reason: 'grade rows are noise when nothing was graded',
    );
  });

  testWidgets('a failed session open renders a message, never a white screen', (
    tester,
  ) async {
    final root = traceRoot();
    addTearDown(() => root.deleteSync(recursive: true));

    // Drive ReviewScreen straight at the trace deck: the core refuses the
    // open, and the screen must say so (this was the white screen).
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: ReviewScreen(
          deckPath: '${root.path}/trace.md',
          rootDir: root.path,
          depth: Depth.recall,
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(
      tester.takeException(),
      isNull,
      reason: 'the open failure must be rendered, not thrown',
    );
    expect(find.text("CAN'T OPEN THIS DECK"), findsOneWidget);
    expect(
      find.textContaining('not a trace'),
      findsOneWidget,
      reason: "the core's own reason is shown",
    );
    expect(find.widgetWithText(FilledButton, 'Back'), findsOneWidget);
  });
}
