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
        child: DecoratedBox(
          decoration: ShapeDecoration(
            shape: StadiumBorder(side: BorderSide(color: color, width: 2)),
          ),
        ),
      ),
    );
  }
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
