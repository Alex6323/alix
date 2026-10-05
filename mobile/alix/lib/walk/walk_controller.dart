import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';

import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/walk/walk_models.dart';
import 'package:alix_mobile/walk/walk_port.dart';

class WalkController extends ChangeNotifier {
  factory WalkController({
    required WalkPortFactory factory,
    required String deckPath,
    required String rootDir,
    String? device,
  }) {
    return WalkController._(factory, deckPath, rootDir, device);
  }

  WalkController._(this._factory, this._deckPath, this._rootDir, this._device) {
    _open();
  }

  final WalkPortFactory _factory;
  final String _deckPath;
  final String _rootDir;
  final String? _device;

  WalkPort? _port;
  WalkStateModel? _state;
  String? _openError;
  ReviewChoiceFeedbackModel? _choice;
  ReviewMultiChoiceFeedbackModel? _multiChoice;
  final Set<int> _multiSelected = {};
  final Sketch _sketch = Sketch();
  bool _serverLive = false;

  WalkStateModel get state {
    final state = _state;
    if (state == null) throw StateError('walk is not open');
    return state;
  }

  String? get openError => _openError;
  ReviewChoiceFeedbackModel? get choice => _choice;
  ReviewMultiChoiceFeedbackModel? get multiChoice => _multiChoice;
  Set<int> get multiSelected => Set.unmodifiable(_multiSelected);
  Sketch get sketch => _sketch;
  bool get serverLive => _serverLive;
  bool get hasChoices => state.choices?.isNotEmpty ?? false;
  bool get revealed => state.phase == WalkPhase.answer && !hasChoices;
  ReviewTutorCardModel? get tutorCard => _requirePort().tutorCard;

  void setServerLive(bool live) {
    if (_serverLive == live) return;
    _serverLive = live;
    notifyListeners();
  }

  void choose(int chosen) {
    final port = _requirePort();
    _choice = port.choose(chosen);
    _state = port.state;
    notifyListeners();
  }

  void toggleChoice(int index) {
    if (!_multiSelected.remove(index)) _multiSelected.add(index);
    notifyListeners();
  }

  void submitChoices() {
    final port = _requirePort();
    _multiChoice = port.chooseMulti(_multiSelected.toList());
    _state = port.state;
    notifyListeners();
  }

  void reveal() {
    _state = _requirePort().reveal();
    notifyListeners();
  }

  void next() {
    _install(_requirePort().next());
    notifyListeners();
  }

  void nextWalk() {
    _install(_requirePort().restart());
    notifyListeners();
  }

  void selectSketchTool(SketchTool tool) {
    _sketch.selectTool(tool);
    notifyListeners();
  }

  void sketchBegin(Offset point, PointerDeviceKind kind) {
    _sketch.begin(point, kind);
    notifyListeners();
  }

  void sketchExtend(Offset point) {
    _sketch.extend(point);
    notifyListeners();
  }

  void sketchEnd() {
    _sketch.end();
    notifyListeners();
  }

  void sketchUndo() {
    _sketch.undo();
    notifyListeners();
  }

  void sketchClear() {
    _sketch.clear();
    notifyListeners();
  }

  String mintTutorCard({required String front, required List<String> back}) {
    return _requirePort().mintTutorCard(front: front, back: back);
  }

  void applyCardNote({required String id, required List<String> notes}) {
    final port = _requirePort();
    port.applyCardNote(id: id, notes: notes);
    _state = port.state;
    notifyListeners();
  }

  WalkPort _requirePort() {
    final port = _port;
    if (port == null) throw StateError('walk is not open');
    return port;
  }

  void _open() {
    try {
      final port = _factory.open(
        deckPath: _deckPath,
        rootDir: _rootDir,
        device: _device,
      );
      _port = port;
      _openError = null;
      _install(port.state);
    } on WalkOpenFailure catch (error) {
      _port = null;
      _state = null;
      _openError = error.message;
    }
  }

  void _install(WalkStateModel next) {
    _state = next;
    _choice = null;
    _multiChoice = null;
    _multiSelected.clear();
    _sketch.reset();
  }
}
