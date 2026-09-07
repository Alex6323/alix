import 'dart:io';

import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync_client.dart' show SyncEntries, SyncPushResult;

/// The seam `SyncController` talks to: the bridge's `paired_*` calls plus
/// the desktop transport, bundled behind one interface so a test can drive
/// a whole cycle without a Rust dylib or a real server.
/// `lib/bridge/sync_bridge.dart` implements it for real;
/// `test/support/fake_sync_port.dart` fakes it. Transport failures and an
/// expired pairing throw `SyncTransportFailure` / `PairingExpired`
/// (`sync_client.dart`), reused rather than mirrored.
abstract class SyncPort {
  /// The pairing's root id, sent as `X-Alix-Root` on every push.
  String get rootId;

  /// The paired root's directory on disk (`<support>/paired/<rootId>/`).
  String get rootDir;

  /// `GET /api/sync/entries`: what the paired desktop currently serves.
  Future<SyncEntries> entries();

  /// Streams [entry]'s zip to [target]. [unpackedBytes] is that entry's
  /// `SyncEntryDto.unpacked_bytes` from a prior `entries()` call. Once the
  /// response's Content-Length is known, and before any byte lands on
  /// disk, the port checks it against the free space at [rootDir] and
  /// throws `SyncFreeSpaceRefusal` instead of starting the write.
  Future<int> pull(String entry, File target, {required int unpackedBytes});

  /// `POST /api/sync/push?deck=<deckId>`.
  Future<SyncPushResult> push(
    String deckId,
    List<int> document, {
    required String pulledRevision,
  });

  /// `paired_recover`: rolls back an interrupted apply and creates
  /// [rootDir] if missing.
  List<String> recover();

  /// `paired_entries`: every entry the phone has pulled, with per-deck
  /// push/conflict state.
  List<SyncEntryState> pairedEntries();

  /// `paired_plan_pushes`: every locally changed, unconflicted deck.
  List<SyncPushPlanItem> planPushes();

  /// `paired_record_push`: commits a push attempt's outcome.
  void recordPush(SyncPushPlanItem item, SyncPushOutcome outcome);

  /// `paired_apply_pull`: unpacks and lands one pulled entry's zip. Runs
  /// off the UI isolate.
  Future<SyncPullReport> applyPull(String entry, String zipPath);

  /// `paired_resolve_conflict`.
  SyncResolution resolveConflict(String deckId, {required bool keepPhone});

  /// `paired_tidy_renamed`.
  List<SyncRenamedEntry> tidyRenamed(List<String> listed);

  /// `paired_remove_entry`.
  void removeEntry(String entry);

  /// `paired_orphans`: entries this phone has a manifest for that [listed]
  /// (the desktop's current `entries()` listing) does not name.
  List<String> pairedOrphans(List<String> listed);

  /// `paired_staging_zip`: the temporary archive path a pull writes [entry]
  /// to and removes once applied.
  String pairedStagingZip(String entry);

  /// The deck's own `title:` for the local copy at [path] (relative to
  /// [rootDir]); null when the phone holds no such file to read one from.
  String? deckTitle(String path);

  /// Releases this port's transport (the bridge port's `HttpSyncClient`
  /// connection pool); safe to call more than once. Called once, from
  /// `SyncController.dispose`.
  void close();
}

/// Thrown by `SyncPort.pull` when the entry would not fit; [needed] and
/// [free] are bytes, folded into `SyncReport.refused` by the controller.
class SyncFreeSpaceRefusal implements Exception {
  const SyncFreeSpaceRefusal({required this.needed, required this.free});

  final int needed;
  final int free;

  @override
  String toString() => 'SyncFreeSpaceRefusal: needs $needed bytes, $free free';
}
