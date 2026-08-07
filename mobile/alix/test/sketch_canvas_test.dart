import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/review/sketch_canvas.dart';

Widget host(Sketch sketch, {bool frozen = false}) {
  return MaterialApp(
    home: Scaffold(
      body: SizedBox(
        width: 300,
        height: 300,
        child: SketchCanvas(
          sketch: sketch,
          ink: const Color(0xFFEEEEEE),
          frozen: frozen,
          onBegin: sketch.begin,
          onExtend: sketch.extend,
          onEnd: sketch.end,
        ),
      ),
    ),
  );
}

void main() {
  testWidgets('a drag records a stroke through the canvas', (tester) async {
    final sketch = Sketch();
    await tester.pumpWidget(host(sketch));

    final gesture = await tester.startGesture(
      const Offset(50, 50),
      kind: PointerDeviceKind.touch,
    );
    await gesture.moveTo(const Offset(80, 90));
    await gesture.moveTo(const Offset(120, 140));
    await gesture.up();
    await tester.pump();

    expect(sketch.strokes, hasLength(1));
    expect(sketch.strokes.single.points.length, greaterThanOrEqualTo(3));
  });

  testWidgets('a palm resting after a stylus does not ink', (tester) async {
    final sketch = Sketch();
    await tester.pumpWidget(host(sketch));

    final pen = await tester.startGesture(
      const Offset(40, 40),
      kind: PointerDeviceKind.stylus,
    );
    await pen.moveTo(const Offset(60, 60));
    await pen.up();

    final palm = await tester.startGesture(
      const Offset(200, 200),
      kind: PointerDeviceKind.touch,
    );
    await palm.moveTo(const Offset(220, 220));
    await palm.up();
    await tester.pump();

    expect(sketch.strokes, hasLength(1), reason: 'only the stylus may draw');
    expect(sketch.strokes.single.points.first, const Offset(40, 40));
  });

  testWidgets('a finger draws on a card that has seen no stylus', (tester) async {
    final sketch = Sketch();
    await tester.pumpWidget(host(sketch));

    final finger = await tester.startGesture(
      const Offset(10, 10),
      kind: PointerDeviceKind.touch,
    );
    await finger.moveTo(const Offset(40, 40));
    await finger.up();
    await tester.pump();

    expect(sketch.strokes, hasLength(1));
  });

  testWidgets('a frozen canvas takes no further marks', (tester) async {
    final sketch = Sketch();
    await tester.pumpWidget(host(sketch, frozen: true));

    final gesture = await tester.startGesture(const Offset(50, 50));
    await gesture.moveTo(const Offset(90, 90));
    await gesture.up();
    await tester.pump();

    expect(sketch.isEmpty, isTrue);
  });

  testWidgets('a drag inside a scroll view draws instead of scrolling', (tester) async {
    final sketch = Sketch();
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: SingleChildScrollView(
            child: Column(
              children: [
                const SizedBox(height: 400),
                SizedBox(
                  width: 300,
                  height: 300,
                  child: SketchCanvas(
                    sketch: sketch,
                    ink: const Color(0xFFEEEEEE),
                    onBegin: sketch.begin,
                    onExtend: sketch.extend,
                    onEnd: sketch.end,
                  ),
                ),
                const SizedBox(height: 400),
              ],
            ),
          ),
        ),
      ),
    );

    final canvas = find.byType(SketchCanvas);
    await tester.drag(canvas, const Offset(0, -60));
    await tester.pump();

    expect(sketch.strokes, isNotEmpty, reason: 'the scroll view must not steal the stroke');
  });
}
