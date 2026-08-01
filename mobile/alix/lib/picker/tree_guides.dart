import 'package:flutter/material.dart';

class TreeGuides extends StatelessWidget {
  const TreeGuides({super.key, required this.tree, required this.color});

  final String tree;
  final Color color;

  static const double _column = 15;

  @override
  Widget build(BuildContext context) {
    final columns = tree.length ~/ 3;
    return SizedBox(
      width: columns * _column,
      child: CustomPaint(
        painter: TreeGuidesPainter(tree: tree, color: color),
      ),
    );
  }
}

class TreeGuidesPainter extends CustomPainter {
  TreeGuidesPainter({required this.tree, required this.color});

  final String tree;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = color
      ..strokeWidth = 1.1
      ..style = PaintingStyle.stroke;
    const column = TreeGuides._column;
    final midY = size.height / 2;
    for (var i = 0; i * 3 + 3 <= tree.length; i++) {
      final segment = tree[i * 3];
      final x = i * column + column / 2;
      if (segment == '│' || segment == '├') {
        canvas.drawLine(Offset(x, 0), Offset(x, size.height), paint);
      } else if (segment == '└') {
        canvas.drawLine(Offset(x, 0), Offset(x, midY), paint);
      }
      if (segment == '├' || segment == '└') {
        canvas.drawLine(
          Offset(x, midY),
          Offset(i * column + column, midY),
          paint,
        );
      }
    }
  }

  @override
  bool shouldRepaint(TreeGuidesPainter old) =>
      old.tree != tree || old.color != color;
}
