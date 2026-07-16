// Regression tests for the hashcards white screen (2026-07-16): a trace deck
// (`% trace:`) has no mobile walk, so ReviewSession.open refuses it; the
// picker used to offer the row anyway and the refusal rendered as a white
// screen. The deck is a trimmed copy of the workspace's real 07-how-a-c.txt.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

const hashcardsTraceDeck = '''
% trace: How a `C
% source: assets
% origin: /home/me/dev/developer/eudoxia0/hashcards

# A `ReadingCloze` block just ended. What is handed to `parse_cloze_cards`?
	The accumulated cloze `text` plus the block's start and end line numbers.
	% at: 55.rs from src/parser.rs:406-447

# Why does this scan use bytes rather than chars?
	Cloze `start`/`end` are byte positions, so counting must be in bytes.
	% at: 57.rs from src/parser.rs:470-529
''';

Directory traceRoot() {
  final root = Directory.systemTemp.createTempSync('alix-hashcards-');
  File('${root.path}/07-how-a-c.txt').writeAsStringSync(hashcardsTraceDeck);
  File('${root.path}/facts.txt')
      .writeAsStringSync('% title: Facts\n# q?\n\ta\n');
  return root;
}

void main() {
  setUpAll(() async => RustLib.init());

  testWidgets('the picker marks a trace deck and never opens a review on it',
      (tester) async {
    final root = traceRoot();
    addTearDown(() => root.deleteSync(recursive: true));

    await tester.pumpWidget(MaterialApp(
      theme: alixDark(),
      home: PickerScreen(root: root.path),
    ));
    await tester.pumpAndSettle();

    expect(find.text('How a `C'), findsOneWidget);
    expect(find.text('trace'), findsOneWidget, reason: 'the row is marked');

    await tester.tap(find.text('How a `C'));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    expect(find.byType(ReviewScreen), findsNothing,
        reason: 'a trace deck must not open a review session');
    expect(find.textContaining('live in the web app'), findsOneWidget,
        reason: 'the tap explains itself instead of doing nothing');
  });

  testWidgets('a failed session open renders a message, never a white screen',
      (tester) async {
    final root = traceRoot();
    addTearDown(() => root.deleteSync(recursive: true));

    // Drive ReviewScreen straight at the trace deck: the core refuses the
    // open, and the screen must say so (this was the white screen).
    await tester.pumpWidget(MaterialApp(
      theme: alixDark(),
      home: ReviewScreen(
        deckPath: '${root.path}/07-how-a-c.txt',
        rootDir: root.path,
        depth: Depth.recall,
      ),
    ));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull,
        reason: 'the open failure must be rendered, not thrown');
    expect(find.text("CAN'T OPEN THIS DECK"), findsOneWidget);
    expect(find.textContaining('not a trace'), findsOneWidget,
        reason: "the core's own reason is shown");
    expect(find.widgetWithText(FilledButton, 'Back'), findsOneWidget);
  });
}
