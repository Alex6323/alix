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
  factory SyncController({required SyncPort port}) =>
      SyncController._(port);

  SyncController._(this._port);

  final SyncPort _port;

  bool _running = false;
  String? _runningEntry;
  SyncReport? _lastReport;
  bool _reportUnread = false;

  bool get running => _running;
  SyncReport? get lastReport => _lastReport;
  bool get reportUnread => _reportUnread;

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
    notifyListeners();
  }

  /// Every deck that currently needs a conflict choice, read live from the
  /// port (not from a past report) so it reflects conflicts left over from
  /// an earlier session too.
  List<SyncPendingConflict> get pendingConflicts => [
    for (final entry in _port.pairedEntries())
      for (final deck in entry.decks)
        if (deck.conflict case final conflict?)
          SyncPendingConflict(
            deckId: deck.deckId,
            entry: entry.entry,
            path: deck.path,
            conflict: conflict,
          ),
  ];

  /// Runs one sync cycle: lists what the desktop serves, pushes every
  /// locally changed deck in order, pulls [entry] (or, when null, every
  /// entry the phone already has a manifest for), tidies renamed entries,
  /// and reports the result. A cycle already running is left alone.
  Future<void> cycle({String? entry}) async {
    if (_running) return;
    _running = true;
    _runningEntry = entry;
    notifyListeners();
    final report = await _runCycle(entry);
    _running = false;
    _runningEntry = null;
    _lastReport = report;
    _reportUnread = true;
    notifyListeners();
  }

  Future<SyncReport> _runCycle(String? onlyEntry) async {
    final landed = <String>[];
    final kept = <String>[];
    final conflicts = <String>[];
    final phoneOnly = <String>[];
    final removed = <String>[];
    final refused = <String>[];

    SyncReport aborted(String error) => SyncReport(
      landed: landed,
      kept: kept,
      conflicts: conflicts,
      phoneOnly: phoneOnly,
      removed: removed,
      refused: refused,
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

    final pushLabels = _deckLabels();
    for (final item in _port.planPushes()) {
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
          break;
        case SyncPushConflict():
          conflicts.add(label);
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
          '$name: needs ${_humanBytes(error.needed)}, '
          '${_humanBytes(error.free)} free',
        );
        continue;
      } on PairingExpired {
        return aborted(syncPairingExpiredMessage);
      } on SyncTransportFailure catch (error) {
        return aborted('could not pull $name: status ${error.status}');
      }
      final entryLabels = _deckLabelsFor(name);
      landed.addAll(
        pullReport.landed.map((id) => entryLabels[id] ?? '$name/$id'),
      );
      kept.addAll(pullReport.kept.map((id) => entryLabels[id] ?? '$name/$id'));
      conflicts.addAll(
        pullReport.conflicts.map((id) => entryLabels[id] ?? '$name/$id'),
      );
      phoneOnly.addAll(pullReport.phoneOnly.map((rel) => '$name/$rel'));
      removed.addAll(pullReport.removed.map((rel) => '$name/$rel'));
    }

    final renamed = [
      for (final r in _port.tidyRenamed([
        for (final e in desktop.entries) e.name,
      ]))
        '${r.old} → ${r.new_}',
    ];

    final knownEntryNames = _port.pairedEntries().map((e) => e.entry).toSet();
    final desktopNames = desktop.entries.map((e) => e.name).toSet();
    final orphaned = [
      for (final name in knownEntryNames)
        if (!desktopNames.contains(name)) name,
    ];
    final leftOut = [
      for (final name in desktopNames)
        if (!knownEntryNames.contains(name)) name,
    ];

    return SyncReport(
      landed: landed,
      kept: kept,
      conflicts: conflicts,
      phoneOnly: phoneOnly,
      removed: removed,
      leftOut: leftOut,
      renamed: renamed,
      orphaned: orphaned,
      refused: refused,
    );
  }

  Future<SyncPullReport> _pullEntry(String entry, int unpackedBytes) async {
    final zipPath = '${_port.rootDir}/.alix/staging/$entry.zip';
    final zipFile = File(zipPath);
    await zipFile.parent.create(recursive: true);
    await _port.pull(entry, zipFile, unpackedBytes: unpackedBytes);
    final report = await _port.applyPull(entry, zipPath);
    if (await zipFile.exists()) await zipFile.delete();
    return report;
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
  Future<void> pushOne(String deckId) async {
    final items = _port.planPushes().where((i) => i.deckId == deckId);
    if (items.isEmpty) return;
    try {
      await _attemptPush(items.first);
    } on PairingExpired {
      return;
    } on SyncTransportFailure {
      return;
    }
    notifyListeners();
  }

  /// Acts on a conflict choice: `Push` pushes now with the returned base;
  /// `Pull` pulls the returned entry; `Done` means the lib already applied
  /// the choice.
  Future<void> resolve(String deckId, {required bool keepPhone}) async {
    final resolution = _port.resolveConflict(deckId, keepPhone: keepPhone);
    switch (resolution) {
      case SyncResolutionDone():
        notifyListeners();
      case SyncResolutionPush(:final item):
        try {
          await _attemptPush(item);
        } on PairingExpired {
          // Silent, matching pushOne: the picker's next cycle re-surfaces it.
        } on SyncTransportFailure {
          // Silent, matching pushOne.
        }
        notifyListeners();
      case SyncResolutionPull(:final entry):
        await _resolvePull(entry);
        notifyListeners();
    }
  }

  Future<void> _resolvePull(String entry) async {
    try {
      final desktop = await _port.entries();
      final desktopEntry = desktop.entries.where((e) => e.name == entry);
      if (desktopEntry.isEmpty) return;
      final pullReport = await _pullEntry(
        entry,
        desktopEntry.first.unpackedBytes,
      );
      final entryLabels = _deckLabelsFor(entry);
      _lastReport = SyncReport(
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
        phoneOnly: [for (final rel in pullReport.phoneOnly) '$entry/$rel'],
        removed: [for (final rel in pullReport.removed) '$entry/$rel'],
      );
      _reportUnread = true;
    } on PairingExpired {
      _lastReport = const SyncReport(error: syncPairingExpiredMessage);
      _reportUnread = true;
    } on SyncTransportFailure catch (error) {
      _lastReport = SyncReport(
        error: 'could not pull $entry: status ${error.status}',
      );
      _reportUnread = true;
    } on SyncFreeSpaceRefusal catch (error) {
      _lastReport = SyncReport(
        refused: [
          '$entry: needs ${_humanBytes(error.needed)}, '
              '${_humanBytes(error.free)} free',
        ],
      );
      _reportUnread = true;
    }
  }

  /// Removes an orphaned entry's local copy (the report sheet's Remove
  /// action); leaving it alone (Keep) needs no call at all.
  Future<void> removeOrphan(String entry) async {
    _port.removeEntry(entry);
    notifyListeners();
  }

  Map<String, String> _deckLabels() {
    return {
      for (final entry in _port.pairedEntries())
        for (final deck in entry.decks)
          deck.deckId: '${entry.entry}/${_basename(deck.path)}',
    };
  }

  Map<String, String> _deckLabelsFor(String entryName) {
    for (final entry in _port.pairedEntries()) {
      if (entry.entry != entryName) continue;
      return {
        for (final deck in entry.decks)
          deck.deckId: '${entry.entry}/${_basename(deck.path)}',
      };
    }
    return const {};
  }
}

String _basename(String path) => path.split('/').last;

String _humanBytes(int bytes) {
  if (bytes >= 1024 * 1024) {
    return '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MB';
  }
  if (bytes >= 1024) return '${(bytes / 1024).toStringAsFixed(1)} KB';
  return '$bytes bytes';
}
