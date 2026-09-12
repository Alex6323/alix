// The one shared `SyncPort` test double, mirroring fake_server_client.dart's
// stance: no network, no Rust dylib. Every capability is a nullable closure
// hook defaulting to a harmless no-op, plus a spy list a test can assert call
// order and arguments from. Real dart:io files still change hands (`pull`
// writes to `target`, `push` reads from `SyncPushPlanItem.document`) since
// that mirrors the real port's contract; tests point `document` at a real
// temp file.
import 'dart:io';

import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync/sync_port.dart';
import 'package:alix_mobile/sync_client.dart';

class FakeSyncPort implements SyncPort {
  FakeSyncPort({this.rootId = 'root-test0000000000000000000000', required this.rootDir});

  @override
  final String rootId;
  @override
  final String rootDir;

  Future<SyncEntries> Function()? entriesImpl;
  final List<int> entriesCalls = [];

  @override
  Future<SyncEntries> entries() async {
    entriesCalls.add(entriesCalls.length);
    final impl = entriesImpl;
    if (impl == null) return SyncEntries(rootId: rootId, entries: const []);
    return impl();
  }

  Future<int> Function(String entry, File target, int unpackedBytes)? pullImpl;
  final List<String> pullCalls = [];

  @override
  Future<int> pull(
    String entry,
    File target, {
    required int unpackedBytes,
  }) async {
    pullCalls.add(entry);
    final impl = pullImpl;
    if (impl == null) {
      await target.writeAsBytes(const []);
      return 0;
    }
    return impl(entry, target, unpackedBytes);
  }

  Future<SyncPushResult> Function(
    String deckId,
    List<int> document,
    String pulledRevision,
  )?
  pushImpl;
  final List<String> pushCalls = [];

  @override
  Future<SyncPushResult> push(
    String deckId,
    List<int> document, {
    required String pulledRevision,
  }) async {
    pushCalls.add(deckId);
    final impl = pushImpl;
    if (impl == null) {
      return SyncPushAccepted(deckId: deckId, revision: 1);
    }
    return impl(deckId, document, pulledRevision);
  }

  List<String> Function()? recoverImpl;

  @override
  List<String> recover() => recoverImpl?.call() ?? const [];

  List<SyncEntryState> Function()? pairedEntriesImpl;

  @override
  List<SyncEntryState> pairedEntries() => pairedEntriesImpl?.call() ?? const [];

  List<SyncPushPlanItem> Function()? planPushesImpl;

  @override
  List<SyncPushPlanItem> planPushes() => planPushesImpl?.call() ?? const [];

  final List<(SyncPushPlanItem, SyncPushOutcome)> recordPushCalls = [];

  @override
  void recordPush(SyncPushPlanItem item, SyncPushOutcome outcome) {
    recordPushCalls.add((item, outcome));
  }

  Future<SyncPullReport> Function(String entry, String zipPath)? applyPullImpl;
  final List<String> applyPullCalls = [];

  @override
  Future<SyncPullReport> applyPull(String entry, String zipPath) async {
    applyPullCalls.add(entry);
    final impl = applyPullImpl;
    if (impl == null) {
      return SyncPullReport(
        entry: entry,
        kind: 'workspace',
        landed: const [],
        kept: const [],
        conflicts: const [],
        phoneOnly: const [],
        removed: const [],
      );
    }
    return impl(entry, zipPath);
  }

  SyncResolution Function(String deckId, bool keepPhone)? resolveConflictImpl;
  final List<String> resolveConflictCalls = [];

  @override
  SyncResolution resolveConflict(String deckId, {required bool keepPhone}) {
    resolveConflictCalls.add(deckId);
    return resolveConflictImpl?.call(deckId, keepPhone) ??
        const SyncResolutionDone();
  }

  List<SyncRenamedEntry> Function(List<String> listed)? tidyRenamedImpl;
  List<String>? tidyRenamedListed;

  @override
  List<SyncRenamedEntry> tidyRenamed(List<String> listed) {
    tidyRenamedListed = listed;
    return tidyRenamedImpl?.call(listed) ?? const [];
  }

  final List<String> removeEntryCalls = [];

  @override
  void removeEntry(String entry) => removeEntryCalls.add(entry);

  List<String> Function(List<String> listed)? pairedOrphansImpl;

  // Defaults to the same "known minus listed" rule the lib now owns, so a
  // test that never sets pairedOrphansImpl (nearly all of them) keeps
  // seeing orphans derived from pairedEntriesImpl exactly as before.
  @override
  List<String> pairedOrphans(List<String> listed) {
    final impl = pairedOrphansImpl;
    if (impl != null) return impl(listed);
    final known = pairedEntries().map((e) => e.entry).toSet();
    final listedSet = listed.toSet();
    return [for (final name in known) if (!listedSet.contains(name)) name];
  }

  String Function(String entry)? pairedStagingZipImpl;

  @override
  String pairedStagingZip(String entry) =>
      pairedStagingZipImpl?.call(entry) ?? '$rootDir/.alix/staging/$entry.zip';

  String? Function(String path)? deckTitleImpl;

  @override
  String? deckTitle(String path) => deckTitleImpl?.call(path);

  int closeCalls = 0;

  @override
  void close() => closeCalls++;
}
