import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';

enum SketchTool { pen, eraser }

/// One continuous mark. Mirrors the web canvas's `{tool, points}` so both
/// clients describe a sketch the same way (web/alix/review/study.js).
@immutable
class Stroke {
  const Stroke(this.tool, this.points);

  final SketchTool tool;
  final List<Offset> points;

  Stroke extendedTo(Offset point) =>
      Stroke(tool, List.unmodifiable([...points, point]));
}

/// The strokes of one card, plus which tool is drawing and whether a stylus
/// has claimed the surface.
///
/// Lives on the review controller rather than the canvas widget: a phone
/// rebuilds on rotation, notification, and app switch, and a sketch lost
/// mid-card is worse there than on a desktop.
class Sketch {
  final List<Stroke> _strokes = [];
  SketchTool _tool = SketchTool.pen;
  Stroke? _live;
  bool _stylusSeen = false;

  List<Stroke> get strokes => List.unmodifiable([..._strokes, ?_live]);
  SketchTool get tool => _tool;
  bool get isEmpty => _strokes.isEmpty && _live == null;

  void selectTool(SketchTool tool) => _tool = tool;

  /// Whether a pointer of this kind may draw. A stylus latches the surface for
  /// the rest of the card, so a palm resting on the glass cannot ink. Latching
  /// per card, not per session: the next card starts open.
  bool accepts(PointerDeviceKind kind) {
    if (kind == PointerDeviceKind.stylus || kind == PointerDeviceKind.invertedStylus) {
      return true;
    }
    return !_stylusSeen;
  }

  void begin(Offset point, PointerDeviceKind kind) {
    if (kind == PointerDeviceKind.stylus || kind == PointerDeviceKind.invertedStylus) {
      _stylusSeen = true;
    }
    if (!accepts(kind)) return;
    _live = Stroke(_tool, List.unmodifiable([point]));
  }

  void extend(Offset point) {
    final live = _live;
    if (live == null) return;
    _live = live.extendedTo(point);
  }

  void end() {
    final live = _live;
    _live = null;
    if (live == null || live.points.length < 2) return;
    _strokes.add(live);
  }

  void undo() {
    if (_strokes.isNotEmpty) _strokes.removeLast();
  }

  void clear() => _strokes.clear();

  void reset() {
    _strokes.clear();
    _live = null;
    _stylusSeen = false;
    _tool = SketchTool.pen;
  }
}
