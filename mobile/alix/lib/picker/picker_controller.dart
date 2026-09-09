import 'package:flutter/foundation.dart';

import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/picker_port.dart';

const bool kAlixProfile = bool.fromEnvironment('ALIX_PROFILE');

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
  PickerDeadline? _deadline;
  bool _serverReachable = false;
  String? _pairedRootDir;
  List<PickerEntry> _pairedRootEntries = const [];

  List<PickerEntry> get entries => _entries;
  PickerDeadline? get deadline => _deadline;
  bool get serverReachable => _serverReachable;
  bool get isMasteredView => _masteredEntries != null;

  /// The active paired desktop's own top-level entries, listed alongside
  /// [entries] rather than instead of them: non-empty only once
  /// [setPairedRoot] names a directory.
  List<PickerEntry> get pairedRootEntries => _pairedRootEntries;

  void setServerReachable(bool reachable) {
    if (_serverReachable == reachable) return;
    _serverReachable = reachable;
    notifyListeners();
  }

  /// Names the paired desktop's own directory to list alongside [entries];
  /// null clears it (no active pairing, or this is not the root screen). A
  /// no-op dir is not re-listed.
  void setPairedRoot(String? dir) {
    if (_pairedRootDir == dir) return;
    _pairedRootDir = dir;
    reload();
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
      _entries = List.unmodifiable(
        _timed(
          'root',
          () => _port.listRoot(_root, profile: kAlixProfile),
        ).entries,
      );
      final pairedRootDir = _pairedRootDir;
      _pairedRootEntries = pairedRootDir == null
          ? const []
          : List.unmodifiable(
              _timed(
                'paired-root',
                () => _port.listRoot(pairedRootDir, profile: kAlixProfile),
              ).entries,
            );
      return;
    }
    final listing = _timed(
      'members',
      () => _port.listMembers(root: _root, dir: dir, profile: kAlixProfile),
    );
    _entries = List.unmodifiable(listing.entries);
    _deadline = listing.deadline;
  }

  PickerListing _timed(String row, PickerListing Function() call) {
    if (!kAlixProfile) return call();
    final stopwatch = Stopwatch()..start();
    final listing = call();
    stopwatch.stop();
    final profile = listing.profile;
    final counters = profile == null
        ? ''
        : profile.counters.map((c) => ' ${c.$1}=${c.$2}').join();
    debugPrint(
      'alix-profile $row bridge_ms=${stopwatch.elapsedMilliseconds} '
      'lib_ms=${profile?.libMs ?? -1}$counters',
    );
    return listing;
  }
}
