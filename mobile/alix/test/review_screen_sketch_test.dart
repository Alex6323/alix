import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch_canvas.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

import 'support/deck_fixture.dart';

void main() {
  setUpAll(() async => RustLib.init());

  Directory tempDir(String prefix) {
    final dir = Directory.systemTemp.createTempSync(prefix);
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  Future<void> pumpDeck(WidgetTester tester, String body, {String name = 'draw.md'}) async {
    final root = tempDir('alix-sketch-');
    final deck = '${root.path}/$name';
    writeTestDeck(deck, body);
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: ReviewScreen(
          key: UniqueKey(),
          deckPath: deck,
          rootDir: root.path,
          depth: ReviewDepth.recall,
          supportDir: tempDir('alix-sketch-support-'),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  /// The law that fails before this feature exists: a draw card took the
  /// typing branch and asked the learner to type what the deck says cannot
  /// be typed.
  testWidgets('a draw card offers a canvas and no text field', (tester) async {
    await pumpDeck(tester, '## Draw the hiragana for "ka".\nか\n<!-- input: draw -->\n');

    expect(find.byType(SketchCanvas), findsOneWidget);
    expect(find.byType(TextField), findsNothing);
  });

  testWidgets('a span cut from a formula draws without any directive', (tester) async {
    await pumpDeck(
      tester,
      '## The quadratic formula\n---\n\$x = -b \\pm \\sqrt{d}\$\n<!-- blank: span hidden="d" b:a1b2c3 -->\n',
      name: 'formula.md',
    );

    expect(
      find.byType(SketchCanvas),
      findsOneWidget,
      reason: 'the lib resolves input: draw for a math span; the client must honour it',
    );
  });

  testWidgets('a typed card is untouched by the draw branch', (tester) async {
    await pumpDeck(tester, '## What is 2 + 2?\n4\n', name: 'typed.md');

    expect(find.byType(SketchCanvas), findsNothing);
  });


  testWidgets('the tool row offers pen, eraser, undo and clear', (tester) async {
    await pumpDeck(tester, '## Draw it\nか\n<!-- input: draw -->\n');

    expect(find.text('Pen'), findsOneWidget);
    expect(find.text('Eraser'), findsOneWidget);
    expect(find.text('Undo'), findsOneWidget);
    expect(find.text('Clear'), findsOneWidget);
  });

  /// Fails against any implementation that holds strokes in the widget.
  testWidgets('a rebuild keeps the strokes', (tester) async {
    await pumpDeck(tester, '## Draw it\nか\n<!-- input: draw -->\n');

    final canvas = find.byType(SketchCanvas);
    final gesture = await tester.startGesture(
      tester.getCenter(canvas),
      kind: PointerDeviceKind.touch,
    );
    await gesture.moveBy(const Offset(20, 20));
    await gesture.moveBy(const Offset(20, -10));
    await gesture.up();
    await tester.pump();

    expect(tester.widget<SketchCanvas>(canvas).sketch.strokes, hasLength(1));

    // A rotation: the widget tree rebuilds under a new size while the screen's
    // state survives, which is where the strokes have to have been kept.
    final view = tester.view;
    addTearDown(view.resetPhysicalSize);
    view.physicalSize = Size(view.physicalSize.height, view.physicalSize.width);
    await tester.pumpAndSettle();

    expect(
      tester.widget<SketchCanvas>(find.byType(SketchCanvas)).sketch.strokes,
      hasLength(1),
      reason: 'strokes held in the widget would be gone here',
    );
  });

  testWidgets('revealing keeps the attempt beside the answer', (tester) async {
    await pumpDeck(tester, '## Draw the hiragana for "ka".\nか\n<!-- input: draw -->\n');

    final canvas = find.byType(SketchCanvas);
    final gesture = await tester.startGesture(tester.getCenter(canvas));
    await gesture.moveBy(const Offset(30, 30));
    await gesture.up();
    await tester.pumpAndSettle();

    await tester.tap(find.text('Reveal'));
    await tester.pumpAndSettle();

    expect(find.byType(SketchCanvas), findsOneWidget, reason: 'the attempt stays for comparison');
    expect(tester.widget<SketchCanvas>(find.byType(SketchCanvas)).frozen, isTrue);
    expect(find.text('か'), findsOneWidget, reason: 'and the answer appears beside it');
  });
}
