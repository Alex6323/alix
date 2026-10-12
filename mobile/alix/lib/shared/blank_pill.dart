import 'package:flutter/material.dart';

class BlankPill extends StatelessWidget {
  const BlankPill({
    super.key,
    required this.asked,
    required this.fontSize,
    required this.color,
  });

  static const widthEm = 5.0;
  static const heightEm = 1.2;
  static const strokeWidth = 2.0;
  static const askedFillAlpha = 0.16;

  final bool asked;
  final double fontSize;
  final Color color;

  @override
  Widget build(BuildContext context) {
    final em = MediaQuery.textScalerOf(context).scale(fontSize);
    return Semantics(
      container: true,
      label: asked ? 'blank' : 'hidden blank',
      child: SizedBox(
        width: em * widthEm,
        height: em * heightEm,
        child: asked
            ? DecoratedBox(
                decoration: ShapeDecoration(
                  color: color.withValues(alpha: askedFillAlpha),
                  shape: StadiumBorder(
                    side: BorderSide(color: color, width: strokeWidth),
                  ),
                ),
              )
            : CustomPaint(painter: DashedStadiumPainter(color)),
      ),
    );
  }
}

class DashedStadiumPainter extends CustomPainter {
  DashedStadiumPainter(this.color);

  static const dash = 5.0;
  static const gap = 3.5;

  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final rect = (Offset.zero & size).deflate(BlankPill.strokeWidth / 2);
    final outline = Path()
      ..addRRect(
        RRect.fromRectAndRadius(rect, Radius.circular(rect.height / 2)),
      );
    final paint = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = BlankPill.strokeWidth;
    for (final metric in outline.computeMetrics()) {
      for (var start = 0.0; start < metric.length; start += dash + gap) {
        canvas.drawPath(metric.extractPath(start, start + dash), paint);
      }
    }
  }

  @override
  bool shouldRepaint(DashedStadiumPainter oldDelegate) =>
      oldDelegate.color != color;
}

WidgetSpan blankPillSpan({
  required bool asked,
  required double fontSize,
  required Color color,
}) {
  return WidgetSpan(
    alignment: PlaceholderAlignment.middle,
    child: BlankPill(asked: asked, fontSize: fontSize, color: color),
  );
}
