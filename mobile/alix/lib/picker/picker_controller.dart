import 'package:flutter/foundation.dart';

import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/picker_port.dart';
import 'package:alix_mobile/profile.dart';


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
    reload();
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
  bool _disposed = false;
  bool _listedOnce = false;
  Future<void>? _inFlight;
  bool _reloadWanted = false;

  List<PickerEntry> get entries => _entries;

  /// True until the first listing has answered; the view shows nothing in
  /// place of the list rather than the empty hint.
  bool get isLoading => !_listedOnce;
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
    if (_inFlight != null) {
      _reloadWanted = true;
      return;
    }
    _inFlight = _load().whenComplete(() {
      _listedOnce = true;
      _inFlight = null;
      if (!_reloadWanted) return;
      _reloadWanted = false;
      reload();
    });
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

  Future<void> _load() async {
    final fixed = _masteredEntries;
    if (fixed != null) {
      _entries = fixed;
      _notify();
      return;
    }
    final dir = _dir;
    if (dir == null) {
      _entries = List.unmodifiable(
        (await _timed(
          'root',
          () => _port.listRoot(_root, profile: kAlixProfile),
        )).entries,
      );
      _notify();
      final pairedRootDir = _pairedRootDir;
      if (pairedRootDir == null) {
        _pairedRootEntries = const [];
        return;
      }
      _pairedRootEntries = List.unmodifiable(
        (await _timed(
          'paired-root',
          () => _port.listRoot(pairedRootDir, profile: kAlixProfile),
        )).entries,
      );
      _notify();
      return;
    }
    final listing = await _timed(
      'members',
      () => _port.listMembers(root: _root, dir: dir, profile: kAlixProfile),
    );
    _entries = List.unmodifiable(listing.entries);
    _deadline = listing.deadline;
    _notify();
  }

  void _notify() {
    _listedOnce = true;
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  Future<PickerListing> _timed(
    String row,
    Future<PickerListing> Function() call,
  ) async {
    if (!kAlixProfile) return call();
    final stopwatch = Stopwatch()..start();
    final listing = await call();
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
