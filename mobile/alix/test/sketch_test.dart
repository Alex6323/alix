import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/sketch.dart';

/// A stroke needs two points to be a mark; one is a tap.
void draw(Sketch sketch, List<Offset> points, {PointerDeviceKind kind = PointerDeviceKind.touch}) {
  sketch.begin(points.first, kind);
  for (final point in points.skip(1)) {
    sketch.extend(point);
  }
  sketch.end();
}

void main() {
  test('a drawn stroke keeps the points it was given', () {
    final sketch = Sketch();
    draw(sketch, const [Offset(1, 1), Offset(2, 2), Offset(3, 3)]);

    expect(sketch.strokes, hasLength(1));
    expect(sketch.strokes.single.points, const [
      Offset(1, 1),
      Offset(2, 2),
      Offset(3, 3),
    ]);
    expect(sketch.strokes.single.tool, SketchTool.pen);
  });

  test('a tap is not a stroke', () {
    final sketch = Sketch();
    draw(sketch, const [Offset(1, 1)]);

    expect(sketch.isEmpty, isTrue);
  });

  test('undo drops the last stroke and clear drops every stroke', () {
    final sketch = Sketch();
    draw(sketch, const [Offset(0, 0), Offset(1, 1)]);
    draw(sketch, const [Offset(5, 5), Offset(6, 6)]);

    sketch.undo();
    expect(sketch.strokes, hasLength(1));
    expect(sketch.strokes.single.points.first, const Offset(0, 0));

    sketch.clear();
    expect(sketch.isEmpty, isTrue);
  });

  test('an erase is a stroke, so undo restores what it removed', () {
    final sketch = Sketch();
    draw(sketch, const [Offset(0, 0), Offset(1, 1)]);
    sketch.selectTool(SketchTool.eraser);
    draw(sketch, const [Offset(0, 0), Offset(1, 1)]);

    expect(sketch.strokes.last.tool, SketchTool.eraser);
    sketch.undo();
    expect(sketch.strokes, hasLength(1));
    expect(sketch.strokes.single.tool, SketchTool.pen);
  });

  test('a stylus locks out touch for the rest of the card', () {
    final sketch = Sketch();
    draw(sketch, const [Offset(0, 0), Offset(1, 1)], kind: PointerDeviceKind.stylus);
    draw(sketch, const [Offset(9, 9), Offset(8, 8)], kind: PointerDeviceKind.touch);

    expect(sketch.strokes, hasLength(1), reason: 'a resting palm must not ink');
    expect(sketch.strokes.single.points.first, const Offset(0, 0));
  });

  /// The half that a lock which never opens would fail: without it, ignoring
  /// every finger always passes the palm test.
  test('a card that never sees a stylus still draws with a finger', () {
    final sketch = Sketch();
    draw(sketch, const [Offset(0, 0), Offset(1, 1)], kind: PointerDeviceKind.touch);

    expect(sketch.strokes, hasLength(1));
  });

  test('the next card starts unlocked', () {
    final sketch = Sketch();
    draw(sketch, const [Offset(0, 0), Offset(1, 1)], kind: PointerDeviceKind.stylus);
    sketch.reset();
    draw(sketch, const [Offset(2, 2), Offset(3, 3)], kind: PointerDeviceKind.touch);

    expect(sketch.strokes, hasLength(1));
  });

  test('reset clears the strokes and the selected tool', () {
    final sketch = Sketch();
    sketch.selectTool(SketchTool.eraser);
    draw(sketch, const [Offset(0, 0), Offset(1, 1)]);
    sketch.reset();

    expect(sketch.isEmpty, isTrue);
    expect(sketch.tool, SketchTool.pen);
  });
}
