import 'dart:async';

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

  final Map<String, PickerStrip> _strips = {};
  bool _searchOpen = false;
  String _query = '';
  List<PickerSearchHit>? _searchable;
  bool _searchLoading = false;
  bool _searchWanted = false;
  int _searchGeneration = 0;
  int _listingGeneration = 0;

  List<PickerEntry> get entries => _entries;

  PickerStrip? stripFor(String path) => _strips[path];

  bool get searchOpen => _searchOpen;
  bool get isSearching => _query.trim().isNotEmpty;

  List<PickerSearchHit>? get searchHits {
    final searchable = _searchable;
    if (searchable == null) return null;
    return [
      for (final hit in searchable)
        if (pickerMatches(hit.entry.title, _query)) hit,
    ];
  }

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
      if (_searchLoading) _searchGeneration++;
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

  void openSearch() {
    if (_searchOpen) return;
    _searchOpen = true;
    _loadSearchable();
    _notifyIfLive();
  }

  void closeSearch() {
    _searchOpen = false;
    _searchWanted = false;
    _query = '';
    _notifyIfLive();
  }

  void setQuery(String query) {
    if (_query == query) return;
    _query = query;
    _notifyIfLive();
  }

  void _loadSearchable() {
    if (_searchLoading) {
      _searchWanted = true;
      return;
    }
    _searchLoading = true;
    final roots = [_root, ?_pairedRootDir];
    final searchGeneration = ++_searchGeneration;
    unawaited(_finishSearch(roots, searchGeneration));
  }

  Future<void> _finishSearch(List<String> roots, int searchGeneration) async {
    List<PickerSearchHit>? hits;
    try {
      hits = await _port.listSearchable(roots);
    } on Object {
      hits = null;
    }
    _searchLoading = false;
    final current = searchGeneration == _searchGeneration;
    if (hits != null &&
        current &&
        !_searchWanted &&
        _searchOpen &&
        !_disposed) {
      _searchable = List.unmodifiable(hits);
      for (final root in roots) {
        _requestStrips(root, [
          for (final hit in hits)
            if (hit.root == root) hit.entry,
        ], searchGeneration: searchGeneration);
      }
      _notifyIfLive();
    }
    final rerun = _searchWanted && _searchOpen && !_disposed;
    _searchWanted = false;
    if (rerun) _loadSearchable();
  }

  void _requestStrips(
    String root,
    Iterable<PickerEntry> entries, {
    int? searchGeneration,
  }) {
    final decks = [
      for (final entry in entries)
        if (!entry.isWorkspace) entry.path,
    ];
    if (decks.isEmpty) return;
    unawaited(_finishStrips(root, decks, _listingGeneration, searchGeneration));
  }

  Future<void> _finishStrips(
    String root,
    List<String> decks,
    int listingGeneration,
    int? searchGeneration,
  ) async {
    Map<String, PickerStrip> strips;
    try {
      strips = await _port.deckStrips(root: root, decks: decks);
    } on Object {
      return;
    }
    if (_disposed ||
        listingGeneration != _listingGeneration ||
        (searchGeneration != null && searchGeneration != _searchGeneration) ||
        strips.isEmpty) {
      return;
    }
    _strips.addAll(strips);
    _notifyIfLive();
  }

  void _notifyIfLive() {
    if (!_disposed) notifyListeners();
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
    _listingGeneration++;
    final fixed = _masteredEntries;
    if (fixed != null) {
      _entries = fixed;
      _notify();
      _requestStrips(_root, fixed);
      return;
    }
    final dir = _dir;
    if (dir == null) {
      if (_searchOpen) _loadSearchable();
      _entries = List.unmodifiable(
        (await _timed(
          'root',
          () => _port.listRoot(_root, profile: kAlixProfile),
        )).entries,
      );
      _notify();
      _requestStrips(_root, _entries);
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
      _requestStrips(pairedRootDir, _pairedRootEntries);
      return;
    }
    final listing = await _timed(
      'members',
      () => _port.listMembers(root: _root, dir: dir, profile: kAlixProfile),
    );
    _entries = List.unmodifiable(listing.entries);
    _deadline = listing.deadline;
    _notify();
    _requestStrips(_root, _entries);
  }

  void _notify() {
    _listedOnce = true;
    _notifyIfLive();
  }

  @override
  void dispose() {
    _disposed = true;
    _searchWanted = false;
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

bool pickerMatches(String label, String query) {
  final needle = query.trim().toLowerCase();
  return needle.isEmpty || label.toLowerCase().contains(needle);
}
