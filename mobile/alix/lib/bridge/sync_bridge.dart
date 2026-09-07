import 'dart:io';

import 'package:alix_mobile/server_client.dart' show ServerConfig;
import 'package:alix_mobile/src/rust/api/listing.dart' as listing_bridge;
import 'package:alix_mobile/src/rust/api/sync.dart' as bridge;
import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync/sync_port.dart';
import 'package:alix_mobile/sync_client.dart';

/// The paired root's directory on disk (`<support>/paired/<rootId>/`); does
/// not create it (`SyncPort.recover` does, via `paired_recover`).
String pairedRootDirFor({required String support, required String rootId}) {
  return bridge.pairedRootDir(support: support, rootId: rootId);
}

/// Creates [rootDir] if missing and rolls back an interrupted apply; the
/// same call `SyncBridgePort.recover` makes, exposed standalone for a
/// caller (app open, the root switcher) that only needs the directory
/// ready before a `SyncPort` exists yet.
List<String> pairedRecoverFor({required String rootDir}) {
  return bridge.pairedRecover(rootDir: rootDir);
}

/// The real `SyncPort`: dials the paired desktop's `/api/sync/*` transport
/// through [SyncClient] and the phone-local `paired_*` bridge calls through
/// the generated `src/rust/api/sync.dart`. The only file allowed to import
/// that generated API (`test/mobile_architecture_test.dart`).
class SyncBridgePort implements SyncPort {
  SyncBridgePort({
    required ServerConfig config,
    required this.rootDir,
    SyncClient? client,
  }) : rootId = config.rootId,
       _client = client ?? HttpSyncClient(config);

  @override
  final String rootId;

  @override
  final String rootDir;

  final SyncClient _client;

  @override
  Future<SyncEntries> entries() => _client.entries();

  @override
  Future<int> pull(
    String entry,
    File target, {
    required int unpackedBytes,
  }) {
    var checked = false;
    return _client.pull(
      entry,
      target,
      onProgress: (received, total) {
        if (checked) return;
        checked = true;
        final needed = bridge.pairedNeedsSpace(
          compressed: BigInt.from(total),
          unpacked: BigInt.from(unpackedBytes),
        );
        final free = bridge.pairedFreeSpace(path: rootDir);
        if (needed > free) {
          throw SyncFreeSpaceRefusal(needed: needed.toInt(), free: free.toInt());
        }
      },
    );
  }

  @override
  Future<SyncPushResult> push(
    String deckId,
    List<int> document, {
    required String pulledRevision,
  }) {
    return _client.push(
      deckId,
      document,
      rootId: rootId,
      pulledRevision: pulledRevision,
    );
  }

  @override
  List<String> recover() => bridge.pairedRecover(rootDir: rootDir);

  @override
  List<SyncEntryState> pairedEntries() => [
    for (final entry in bridge.pairedEntries(rootDir: rootDir)) _entryState(entry),
  ];

  @override
  List<SyncPushPlanItem> planPushes() => [
    for (final item in bridge.pairedPlanPushes(rootDir: rootDir)) _planItem(item),
  ];

  @override
  void recordPush(SyncPushPlanItem item, SyncPushOutcome outcome) {
    bridge.pairedRecordPush(
      rootDir: rootDir,
      item: _toBridgeItem(item),
      outcome: _toBridgeOutcome(outcome),
    );
  }

  @override
  Future<SyncPullReport> applyPull(String entry, String zipPath) async {
    final report = await bridge.pairedApplyPull(
      rootDir: rootDir,
      entry: entry,
      zipPath: zipPath,
    );
    return SyncPullReport(
      entry: report.entry,
      kind: report.kind,
      landed: report.landed,
      kept: report.kept,
      conflicts: report.conflicts,
      phoneOnly: report.phoneOnly,
      removed: report.removed,
    );
  }

  @override
  SyncResolution resolveConflict(String deckId, {required bool keepPhone}) {
    final resolution = bridge.pairedResolveConflict(
      rootDir: rootDir,
      deckId: deckId,
      keepPhone: keepPhone,
    );
    return resolution.when(
      done: () => const SyncResolutionDone(),
      push: (item) => SyncResolutionPush(_planItem(item)),
      pull: (entry) => SyncResolutionPull(entry),
    );
  }

  @override
  List<SyncRenamedEntry> tidyRenamed(List<String> listed) => [
    for (final renamed in bridge.pairedTidyRenamed(
      rootDir: rootDir,
      listed: listed,
    ))
      SyncRenamedEntry(old: renamed.old, new_: renamed.new_),
  ];

  @override
  void removeEntry(String entry) =>
      bridge.pairedRemoveEntry(rootDir: rootDir, entry: entry);

  @override
  List<String> pairedOrphans(List<String> listed) =>
      bridge.pairedOrphans(rootDir: rootDir, listed: listed);

  @override
  String pairedStagingZip(String entry) =>
      bridge.pairedStagingZip(rootDir: rootDir, entry: entry);

  @override
  String? deckTitle(String path) {
    // [path] is root-relative, the same "loose deck IS its entry" shape
    // `SyncController._deckLabel` only calls this for; `listRoot`'s own
    // `DeckSummary.path` is the absolute filesystem path (`src/listing.rs`),
    // so this compares absolute to absolute the way the lib's own
    // `PairedRoot::entry_root` resolves a loose deck's root as `rootDir`
    // itself.
    final absolute = '$rootDir/$path';
    for (final entry in listing_bridge.listRoot(root: rootDir)) {
      if (entry.path == absolute) return entry.title;
    }
    return null;
  }

  @override
  void close() {
    final client = _client;
    if (client is HttpSyncClient) client.close();
  }
}

SyncEntryState _entryState(bridge.PairedEntryState entry) {
  return SyncEntryState(
    entry: entry.entry,
    kind: entry.kind,
    decks: [for (final deck in entry.decks) _deckState(deck)],
  );
}

SyncDeckState _deckState(bridge.PairedDeckState deck) {
  return SyncDeckState(
    deckId: deck.deckId,
    path: deck.path,
    unpushed: deck.unpushed,
    phoneSaves: deck.phoneSaves.toInt(),
    phoneAtMs: deck.phoneAtMs?.toInt(),
    conflict: deck.conflict == null ? null : _conflict(deck.conflict!),
  );
}

PairedConflict _conflict(bridge.PairedConflict conflict) {
  return conflict.when(
    push: (desktopRevision, pulledRevision, desktopWriter) => PairedConflictPush(
      desktopRevision: desktopRevision?.toInt(),
      pulledRevision: pulledRevision?.toInt(),
      desktopWriter: desktopWriter == null ? null : _writer(desktopWriter),
    ),
    pull: (pulledRevision, pulledWriter) => PairedConflictPull(
      pulledRevision: pulledRevision?.toInt(),
      pulledWriter: pulledWriter == null ? null : _writer(pulledWriter),
    ),
  );
}

SyncWriter _writer(bridge.PairedWriter writer) {
  return SyncWriter(device: writer.device, atMs: writer.atMs.toInt());
}

SyncPushPlanItem _planItem(bridge.PushPlanItem item) {
  return SyncPushPlanItem(
    deckId: item.deckId,
    entry: item.entry,
    document: item.document,
    base: item.base?.toInt(),
    phoneRevision: item.phoneRevision.toInt(),
  );
}

bridge.PushPlanItem _toBridgeItem(SyncPushPlanItem item) {
  return bridge.PushPlanItem(
    deckId: item.deckId,
    entry: item.entry,
    document: item.document,
    base: item.base == null ? null : BigInt.from(item.base!),
    phoneRevision: BigInt.from(item.phoneRevision),
  );
}

bridge.PushOutcomeDto _toBridgeOutcome(SyncPushOutcome outcome) {
  return switch (outcome) {
    SyncPushAcceptedOutcome(:final revision) => bridge.PushOutcomeDto.accepted(
      revision: BigInt.from(revision),
    ),
    SyncPushConflictOutcome(:final desktopRevision, :final desktopWriter) =>
      bridge.PushOutcomeDto.conflict(
        desktopRevision: desktopRevision == null
            ? null
            : BigInt.from(desktopRevision),
        desktopWriter: desktopWriter == null
            ? null
            : bridge.PairedWriter(
                device: desktopWriter.device,
                atMs: BigInt.from(desktopWriter.atMs),
              ),
      ),
    SyncPushNotServedOutcome() => const bridge.PushOutcomeDto.notServed(),
  };
}
