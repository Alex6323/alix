import 'dart:io';

import 'package:flutter/foundation.dart';

import 'package:alix_mobile/server_client.dart' show PairingExpired;
import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync/sync_port.dart';
import 'package:alix_mobile/sync_client.dart';

/// The desktop now serves a different root than this pairing was made
/// against; per docs/API.md section 4.12 the client must stop and re-pair
/// rather than write anything.
const String syncRootMismatchMessage =
    'the desktop now serves another folder; pair it as a new root';

/// Reused everywhere a 401 ends a paired call (matches the wording already
/// shown elsewhere in the app).
const String syncPairingExpiredMessage =
    'Pairing expired. Pair again from Settings → Connected devices.';

/// Owns one active paired root and runs the three sync flows: a full cycle
/// (entries, pushes, pulls, tidy), a single deck's post-review push, and a
/// conflict resolution. No retries, no timers: each flow is a plain
/// sequence over `SyncPort`.
class SyncController extends ChangeNotifier {
  factory SyncController({required SyncPort port, String? initialError}) =>
      SyncController._(port, initialError);

  SyncController._(this._port, String? initialError) {
    _refreshPairedState();
    if (initialError != null) {
      _lastReport = SyncReport(error: initialError);
      _reportUnread = true;
    }
  }

  final SyncPort _port;

  bool _running = false;
  String? _runningEntry;
  SyncReport? _lastReport;
  bool _reportUnread = false;
  bool _disposed = false;

  /// The last successful `entries()` listing this controller made; null
  /// while offline or before the first listing. A root switch always
  /// builds a fresh controller (`picker_screen.dart`), which starts null
  /// again, so switching roots clears this without any code here.
  SyncEntries? _lastListing;

  /// Deck ids the running cycle's own push loop has already attempted
  /// (any outcome), so the drain below skips one a concurrent [pushOne]
  /// queued for the same deck rather than pushing it twice. Empty outside
  /// a cycle.
  final Set<String> _cycleAttemptedPushDeckIds = {};

  /// Deck ids [pushOne] deferred because a cycle was running; drained
  /// (each re-attempted, unless the cycle's own loop already attempted it)
  /// once the cycle ends, so finishing a review mid-cycle is never dropped.
  final Set<String> _pendingPushes = {};

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  /// Notifies listeners unless this controller was disposed while an
  /// awaited call (a push, a pull) was still in flight; a screen can pop
  /// mid-cycle, and the cycle keeps running to completion regardless.
  void _notify() {
    if (_disposed) return;
    notifyListeners();
  }

  bool get running => _running;
  SyncReport? get lastReport => _lastReport;
  bool get reportUnread => _reportUnread;

  /// Every entry the phone has pulled, as of the last guarded scan
  /// ([_refreshPairedState]), never a fresh fallible port call: a caller
  /// resolving a document path to a deck id via `deckIdForPath` (a tap
  /// callback, a listener) must not risk a corrupt state file throwing
  /// through it. Empty before the first scan or after a failed one.
  List<SyncEntryState> get pairedEntries => _pairedEntriesCache;
  List<SyncEntryState> _pairedEntriesCache = const [];

  /// One line for the picker's status readout while a cycle runs or its
  /// last report is unread; null the rest of the time.
  String? get statusLine {
    if (_running) {
      return _runningEntry == null
          ? 'Syncing your decks…'
          : 'Syncing $_runningEntry…';
    }
    if (_reportUnread && _lastReport != null) return _lastReport!.summary();
    return null;
  }

  /// Marks the last report read; the picker's status line clears.
  void markReportRead() {
    if (!_reportUnread) return;
    _reportUnread = false;
    _notify();
  }

  List<SyncPendingConflict> _pendingConflicts = const [];
  Set<String> _unpushedEntries = const {};
  List<SyncEntry> _availableEntries = const [];

  /// Every deck that currently needs a conflict choice: refreshed after
  /// every call that can change it (construction, a cycle, a resolve, a
  /// push, a removal), never scanned fresh from a widget build, so it
  /// reflects conflicts left over from an earlier session too without ever
  /// running the underlying disk scan on the UI thread.
  List<SyncPendingConflict> get pendingConflicts => _pendingConflicts;

  /// Names of paired entries with at least one deck carrying unpushed local
  /// progress, refreshed alongside [pendingConflicts]. The report sheet's
  /// orphan Remove confirmation uses it to name what a removal discards.
  Set<String> get unpushedEntries => _unpushedEntries;

  /// Entries the last successful listing named that this phone has no
  /// manifest for: never pulled, so tapping one (`cycle(entry: name)`)
  /// does a first pull rather than a re-sync. Empty before any listing,
  /// refreshed alongside [pendingConflicts].
  List<SyncEntry> get availableEntries => _availableEntries;

  /// Rebuilds [pendingConflicts], [unpushedEntries], and [availableEntries]
  /// from `_port.pairedEntries()` (one scan) and [_lastListing]. A failed
  /// scan (a corrupt state file) leaves all three empty rather than
  /// throwing: an unresolvable local state is not worth crashing over.
  void _refreshPairedState() {
    final List<SyncEntryState> entries;
    try {
      entries = _port.pairedEntries();
    } on Object {
      _pendingConflicts = const [];
      _unpushedEntries = const {};
      _availableEntries = const [];
      _pairedEntriesCache = const [];
      return;
    }
    _pairedEntriesCache = entries;
    _pendingConflicts = [
      for (final entry in entries)
        for (final deck in entry.decks)
          if (deck.conflict case final conflict?)
            SyncPendingConflict(
              deckId: deck.deckId,
              label: _deckLabel(entry, deck),
              conflict: conflict,
              phoneSaves: deck.phoneSaves,
              phoneAtMs: deck.phoneAtMs,
            ),
    ];
    final knownNames = entries.map((e) => e.entry).toSet();
    _unpushedEntries = {
      for (final entry in entries)
        if (entry.decks.any((deck) => deck.unpushed)) entry.entry,
    };
    final listing = _lastListing;
    _availableEntries = listing == null
        ? const []
        : [
            for (final entry in listing.entries)
              if (!knownNames.contains(entry.name)) entry,
          ];
  }

  /// Runs one sync cycle: lists what the desktop serves, pushes every
  /// locally changed deck in order, pulls [entry] (or, when null, every
  /// entry the phone already has a manifest for), tidies renamed entries,
  /// and reports the result. A cycle already running is left alone.
  Future<void> cycle({String? entry}) async {
    if (_running) return;
    _running = true;
    _runningEntry = entry;
    _notify();
    SyncReport report;
    try {
      report = await _runCycle(entry);
    } on Object catch (error) {
      report = SyncReport(error: 'sync failed: $error');
    } finally {
      _running = false;
      _runningEntry = null;
      final deferred = _pendingPushes.toList();
      _pendingPushes.clear();
      for (final deckId in deferred) {
        if (!_cycleAttemptedPushDeckIds.contains(deckId)) {
          await pushOne(deckId);
        }
      }
      _cycleAttemptedPushDeckIds.clear();
    }
    _refreshPairedState();
    _lastReport = report;
    _reportUnread = !report.isEmpty;
    _notify();
  }

  Future<SyncReport> _runCycle(String? onlyEntry) async {
    final landed = <String>[];
    final kept = <String>[];
    final conflicts = <String>[];
    final conflictDeckIds = <String>[];
    final phoneOnly = <String>[];
    final removed = <String>[];
    final refused = <String>[];
    final pushed = <String>[];

    SyncReport aborted(String error) => SyncReport(
      landed: landed,
      kept: kept,
      conflicts: conflicts,
      conflictDeckIds: conflictDeckIds,
      phoneOnly: phoneOnly,
      removed: removed,
      refused: refused,
      pushed: pushed,
      error: error,
    );

    final SyncEntries desktop;
    try {
      desktop = await _port.entries();
    } on PairingExpired {
      return aborted(syncPairingExpiredMessage);
    } on SyncTransportFailure catch (error) {
      return aborted(
        'could not list what the desktop serves: status ${error.status}',
      );
    }
    if (desktop.rootId != _port.rootId) {
      return aborted(syncRootMismatchMessage);
    }
    _lastListing = desktop;

    final pushLabels = _deckLabels();
    for (final item in _port.planPushes()) {
      _cycleAttemptedPushDeckIds.add(item.deckId);
      final label = pushLabels[item.deckId] ?? '${item.entry}/${item.deckId}';
      final SyncPushResult result;
      try {
        result = await _attemptPush(item);
      } on PairingExpired {
        return aborted(syncPairingExpiredMessage);
      } on SyncTransportFailure catch (error) {
        return aborted('could not push $label: status ${error.status}');
      }
      switch (result) {
        case SyncPushAccepted():
          pushed.add(label);
        case SyncPushConflict():
          conflicts.add(label);
          conflictDeckIds.add(item.deckId);
        case SyncPushRootMismatch():
          return aborted(syncRootMismatchMessage);
        case SyncPushNotServed():
          break;
        case SyncPushTooLarge():
          refused.add('$label: too large to push');
        case SyncPushRejected(:final status):
          refused.add('$label: the desktop refused it (status $status)');
      }
    }

    final toPull = onlyEntry != null
        ? [onlyEntry]
        : [for (final entry in _port.pairedEntries()) entry.entry];
    for (final name in toPull) {
      final desktopEntry = desktop.entries.where((e) => e.name == name);
      if (desktopEntry.isEmpty) continue;
      final SyncPullReport pullReport;
      try {
        pullReport = await _pullEntry(name, desktopEntry.first.unpackedBytes);
      } on SyncFreeSpaceRefusal catch (error) {
        refused.add(
          '$name: needs ${humanBytes(error.needed)}, '
          '${humanBytes(error.free)} free',
        );
        continue;
      } on PairingExpired {
        return aborted(syncPairingExpiredMessage);
      } on SyncTransportFailure catch (error) {
        return aborted('could not pull $name: status ${error.status}');
      } on Object catch (error) {
        return aborted('could not apply $name: $error');
      }
      final entryLabels = _deckLabelsFor(name);
      landed.addAll(
        pullReport.landed.map((id) => entryLabels[id] ?? '$name/$id'),
      );
      kept.addAll(pullReport.kept.map((id) => entryLabels[id] ?? '$name/$id'));
      conflicts.addAll(
        pullReport.conflicts.map((id) => entryLabels[id] ?? '$name/$id'),
      );
      conflictDeckIds.addAll(pullReport.conflicts);
      phoneOnly.addAll(pullReport.phoneOnly.map((rel) => '$name/$rel'));
      removed.addAll(pullReport.removed.map((rel) => '$name/$rel'));
    }

    final renamed = [
      for (final r in _port.tidyRenamed([
        for (final e in desktop.entries) e.name,
      ]))
        '${r.old} → ${r.new_}',
    ];

    final desktopNames = [for (final e in desktop.entries) e.name];
    final orphaned = _port.pairedOrphans(desktopNames);
    final knownEntryNames = _port.pairedEntries().map((e) => e.entry).toSet();
    final desktopNameSet = desktopNames.toSet();
    final notOnPhone = [
      for (final name in desktopNameSet)
        if (!knownEntryNames.contains(name)) name,
    ];
    final leftOut = [
      for (final entry in desktop.entries)
        if (knownEntryNames.contains(entry.name))
          for (final path in entry.leftOut) '${entry.name}/$path',
    ];

    return SyncReport(
      landed: landed,
      kept: kept,
      conflicts: conflicts,
      conflictDeckIds: conflictDeckIds,
      phoneOnly: phoneOnly,
      removed: removed,
      leftOut: leftOut,
      notOnPhone: notOnPhone,
      renamed: renamed,
      orphaned: orphaned,
      refused: refused,
      pushed: pushed,
    );
  }

  Future<SyncPullReport> _pullEntry(String entry, int unpackedBytes) async {
    final zipPath = _port.pairedStagingZip(entry);
    final zipFile = File(zipPath);
    await zipFile.parent.create(recursive: true);
    await _port.pull(entry, zipFile, unpackedBytes: unpackedBytes);
    try {
      return await _port.applyPull(entry, zipPath);
    } finally {
      if (await zipFile.exists()) await zipFile.delete();
    }
  }

  /// Reads [item]'s document, pushes it, and records whatever the lib
  /// tracks for the outcome (accepted revision, conflict mark, or the
  /// not-served no-op); returns the raw transport result for the caller to
  /// react to (a report line, a live conflict).
  Future<SyncPushResult> _attemptPush(SyncPushPlanItem item) async {
    final bytes = await File(item.document).readAsBytes();
    final result = await _port.push(
      item.deckId,
      bytes,
      pulledRevision: item.pulledRevisionHeader,
    );
    switch (result) {
      case SyncPushAccepted(:final revision):
        _port.recordPush(item, SyncPushAcceptedOutcome(revision));
      case SyncPushConflict(:final desktopRevision, :final desktopWriter):
        _port.recordPush(
          item,
          SyncPushConflictOutcome(
            desktopRevision: desktopRevision,
            desktopWriter: desktopWriter,
          ),
        );
      case SyncPushNotServed():
        _port.recordPush(item, const SyncPushNotServedOutcome());
      case SyncPushRootMismatch():
      case SyncPushTooLarge():
      case SyncPushRejected():
        break;
    }
    return result;
  }

  /// Pushes one deck's local progress after a review session ends
  /// (`review_screen.dart`'s post-summary hook). Silent on failure or when
  /// nothing is planned for [deckId]; a conflict is recorded and then
  /// visible through [pendingConflicts], for the summary screen's choice.
  /// A running cycle defers the push instead of dropping it: drained once
  /// the cycle ends, unless its own push loop already attempted this deck,
  /// so a review finished mid-cycle is never dropped nor pushed twice.
  Future<void> pushOne(String deckId) async {
    if (_running) {
      _pendingPushes.add(deckId);
      return;
    }
    final items = _port.planPushes().where((i) => i.deckId == deckId);
    if (items.isEmpty) return;
    try {
      await _attemptPush(items.first);
    } on PairingExpired {
      return;
    } on SyncTransportFailure {
      return;
    }
    _refreshPairedState();
    _notify();
  }

  /// Acts on a conflict choice: `Push` pushes now with the returned base;
  /// `Pull` pulls the returned entry; `Done` means the lib already applied
  /// the choice. No-op for a [deckId] outside [pendingConflicts].
  Future<void> resolve(String deckId, {required bool keepPhone}) async {
    final pending = _pendingConflicts.where((c) => c.deckId == deckId);
    if (pending.isEmpty) return;
    final resolution = _port.resolveConflict(deckId, keepPhone: keepPhone);
    switch (resolution) {
      case SyncResolutionDone():
        break;
      case SyncResolutionPush(:final item):
        try {
          await _attemptPush(item);
        } on PairingExpired {
          // Silent, matching pushOne: the picker's next cycle re-surfaces it.
        } on SyncTransportFailure {
          // Silent, matching pushOne.
        }
      case SyncResolutionPull(:final entry):
        await _resolvePull(entry);
    }
    // `_refreshPairedState` first: a report's conflict rows are trimmed by
    // the deck ids the lib still reports as conflicted, not by matching
    // [deckId] itself, since a fresh conflict (a keep-phone retry that hit
    // another 409) must stay visible.
    _refreshPairedState();
    if (_lastReport case final report?) {
      final stillConflicted = _pendingConflicts.map((c) => c.deckId).toSet();
      _lastReport = report.keepingConflictsIn(stillConflicted);
    }
    _notify();
  }

  Future<void> _resolvePull(String entry) async {
    SyncReport report;
    try {
      final desktop = await _port.entries();
      _lastListing = desktop;
      final desktopEntry = desktop.entries.where((e) => e.name == entry);
      if (desktopEntry.isEmpty) return;
      final pullReport = await _pullEntry(
        entry,
        desktopEntry.first.unpackedBytes,
      );
      final entryLabels = _deckLabelsFor(entry);
      report = SyncReport(
        landed: [
          for (final id in pullReport.landed) entryLabels[id] ?? '$entry/$id',
        ],
        kept: [
          for (final id in pullReport.kept) entryLabels[id] ?? '$entry/$id',
        ],
        conflicts: [
          for (final id in pullReport.conflicts)
            entryLabels[id] ?? '$entry/$id',
        ],
        conflictDeckIds: pullReport.conflicts,
        phoneOnly: [for (final rel in pullReport.phoneOnly) '$entry/$rel'],
        removed: [for (final rel in pullReport.removed) '$entry/$rel'],
      );
    } on PairingExpired {
      report = const SyncReport(error: syncPairingExpiredMessage);
    } on SyncTransportFailure catch (error) {
      report = SyncReport(error: 'could not pull $entry: status ${error.status}');
    } on SyncFreeSpaceRefusal catch (error) {
      report = SyncReport(
        refused: [
          '$entry: needs ${humanBytes(error.needed)}, '
              '${humanBytes(error.free)} free',
        ],
      );
    } on Object catch (error) {
      report = SyncReport(error: 'could not apply $entry: $error');
    }
    _lastReport = report;
    _reportUnread = !report.isEmpty;
  }

  /// Removes an orphaned entry's local copy (the report sheet's Remove
  /// action, after its own confirmation); leaving it alone (Keep) needs no
  /// call at all.
  Future<void> removeOrphan(String entry) async {
    _port.removeEntry(entry);
    if (_lastReport case final report?) _lastReport = report.withoutOrphan(entry);
    _refreshPairedState();
    _notify();
  }

  Map<String, String> _deckLabels() {
    return {
      for (final entry in _port.pairedEntries())
        for (final deck in entry.decks) deck.deckId: _deckLabel(entry, deck),
    };
  }

  Map<String, String> _deckLabelsFor(String entryName) {
    for (final entry in _port.pairedEntries()) {
      if (entry.entry != entryName) continue;
      return {
        for (final deck in entry.decks) deck.deckId: _deckLabel(entry, deck),
      };
    }
    return const {};
  }

  /// A workspace member reads `<entry>/<basename>` (`Biology/cells.md`); a
  /// loose deck IS its entry (`deck.path == entry.entry`), so that shape
  /// would double the name (`greek.md/greek.md`). Its title stands in
  /// instead, falling back to the entry name when the phone holds no local
  /// copy to read one from.
  String _deckLabel(SyncEntryState entry, SyncDeckState deck) {
    if (entry.kind != 'workspace') {
      return _port.deckTitle(deck.path) ?? entry.entry;
    }
    return '${entry.entry}/${_basename(deck.path)}';
  }
}

String _basename(String path) => path.split('/').last;
