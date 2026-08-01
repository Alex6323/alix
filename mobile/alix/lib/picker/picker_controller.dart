import 'package:flutter/foundation.dart';

import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/picker_port.dart';

class PickerController extends ChangeNotifier {
  factory PickerController({
    required PickerPort port,
    required String root,
    String? dir,
    Iterable<PickerEntry>? masteredEntries,
  }) {
    return PickerController._(port, root, dir, masteredEntries);
  }

  PickerController._(
    this._port,
    this._root,
    this._dir,
    Iterable<PickerEntry>? masteredEntries,
  ) : _masteredEntries = masteredEntries == null
          ? null
          : List.unmodifiable(masteredEntries) {
    _load();
  }

  final PickerPort _port;
  final String _root;
  final String? _dir;
  final List<PickerEntry>? _masteredEntries;

  List<PickerEntry> _entries = const [];
  List<String> _conflicts = const [];
  PickerDeadline? _deadline;
  bool _conflictsDismissed = false;
  bool _serverReachable = false;

  List<PickerEntry> get entries => _entries;
  List<String> get conflicts => _conflicts;
  PickerDeadline? get deadline => _deadline;
  bool get conflictsDismissed => _conflictsDismissed;
  bool get serverReachable => _serverReachable;
  bool get isMasteredView => _masteredEntries != null;

  void setServerReachable(bool reachable) {
    if (_serverReachable == reachable) return;
    _serverReachable = reachable;
    notifyListeners();
  }

  void reload() {
    _load();
    notifyListeners();
  }

  void clearDeadline(String dir) {
    _port.setWorkspaceDeadline(dir: dir, date: null);
    reload();
  }

  void setDeadline({required String dir, required String date}) {
    _port.setWorkspaceDeadline(dir: dir, date: date);
    reload();
  }

  void dismissConflicts() {
    if (_conflictsDismissed) return;
    _conflictsDismissed = true;
    notifyListeners();
  }

  Future<void> addTutorial() async {
    await _port.addTutorialDeck(_root);
    reload();
  }

  void _load() {
    final fixed = _masteredEntries;
    if (fixed != null) {
      _entries = fixed;
      return;
    }
    final dir = _dir;
    if (dir == null) {
      _entries = List.unmodifiable(_port.listRoot(_root));
      _conflicts = List.unmodifiable(_port.syncConflicts(_root));
      return;
    }
    _entries = List.unmodifiable(_port.listMembers(root: _root, dir: dir));
    _deadline = _port.workspaceDeadline(root: _root, dir: dir);
  }
}
