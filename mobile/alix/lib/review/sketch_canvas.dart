import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';

import 'package:alix_mobile/review/sketch.dart';

/// The surface an `input: draw` card is answered on.
///
/// `Listener` rather than `GestureDetector`: a drag inside a scrollable answer
/// area would otherwise be claimed by the scroll view, and a stroke would
/// scroll the card instead of drawing on it.
class SketchCanvas extends StatelessWidget {
  const SketchCanvas({
    super.key,
    required this.sketch,
    required this.ink,
    this.onBegin,
    this.onExtend,
    this.onEnd,
    this.frozen = false,
  });

  final Sketch sketch;
  final Color ink;
  final void Function(Offset point, PointerDeviceKind kind)? onBegin;
  final void Function(Offset point)? onExtend;
  final VoidCallback? onEnd;

  /// After reveal the attempt stays on screen for comparison, but takes no
  /// further marks: it is evidence now, not a surface.
  final bool frozen;

  @override
  Widget build(BuildContext context) {
    final painter = CustomPaint(
      painter: SketchPainter(strokes: sketch.strokes, ink: ink),
      size: Size.infinite,
    );
    if (frozen) return painter;
    return Listener(
      behavior: HitTestBehavior.opaque,
      onPointerDown: (event) => onBegin?.call(event.localPosition, event.kind),
      onPointerMove: (event) => onExtend?.call(event.localPosition),
      onPointerUp: (_) => onEnd?.call(),
      onPointerCancel: (_) => onEnd?.call(),
      child: painter,
    );
  }
}

class SketchPainter extends CustomPainter {
  const SketchPainter({required this.strokes, required this.ink});

  static const double penWidth = 2.5;
  static const double eraserWidth = 24;

  final List<Stroke> strokes;
  final Color ink;

  @override
  void paint(Canvas canvas, Size size) {
    // The eraser cuts pixels rather than painting the background colour, so a
    // layer is needed for blend modes to have anything to cut.
    canvas.saveLayer(Offset.zero & size, Paint());
    for (final stroke in strokes) {
      if (stroke.points.length < 2) continue;
      final erasing = stroke.tool == SketchTool.eraser;
      final paint = Paint()
        ..style = PaintingStyle.stroke
        ..strokeCap = StrokeCap.round
        ..strokeJoin = StrokeJoin.round
        ..color = erasing ? const Color(0xFF000000) : ink
        ..strokeWidth = erasing ? eraserWidth : penWidth
        ..blendMode = erasing ? BlendMode.clear : BlendMode.srcOver;
      final path = Path()
        ..moveTo(stroke.points.first.dx, stroke.points.first.dy);
      for (final point in stroke.points.skip(1)) {
        path.lineTo(point.dx, point.dy);
      }
      canvas.drawPath(path, paint);
    }
    canvas.restore();
  }

  @override
  bool shouldRepaint(SketchPainter old) =>
      old.strokes != strokes || old.ink != ink;
}
