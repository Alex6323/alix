import 'package:flutter/foundation.dart';

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
  bool _serverLive = false;

  WalkStateModel get state {
    final state = _state;
    if (state == null) throw StateError('walk session is not open');
    return state;
  }

  String? get openError => _openError;

  bool get serverLive => _serverLive;

  void setServerLive(bool live) {
    if (_serverLive == live) return;
    _serverLive = live;
    notifyListeners();
  }

  void predict(String text) {
    final port = _requirePort();
    port.predict(text);
    _state = port.state;
    notifyListeners();
  }

  void grade(WalkGrade grade) {
    _state = _requirePort().grade(grade);
    notifyListeners();
  }

  void restart() {
    _open();
    notifyListeners();
  }

  int? examCooldownMs(int nowMs) => _requirePort().examCooldownMs(nowMs);

  void applyExamPassed(int nowMs) => _requirePort().applyExamPassed(nowMs);

  void applyExamFailed(int nowMs) => _requirePort().applyExamFailed(nowMs);

  WalkPort _requirePort() {
    final port = _port;
    if (port == null) throw StateError('walk session is not open');
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
      _state = port.state;
      _openError = null;
    } on WalkOpenFailure catch (error) {
      _port = null;
      _state = null;
      _openError = error.message;
    }
  }
}
