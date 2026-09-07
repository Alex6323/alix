// Plain Dart mirrors of the paired-sync bridge DTOs (docs/API.md section
// 4.12, `mobile/alix/rust/src/api/sync.rs`), kept free of generated-bridge
// imports so `sync_controller.dart` and its tests never need the Rust
// dylib. `lib/bridge/sync_bridge.dart` is the only place that converts
// between these and the generated `src/rust/api/sync.dart` types; both the
// transport shapes (`SyncEntries`, `SyncPushResult`, `SyncWriter`) and the
// naming of the types below reuse or avoid colliding with
// `sync_client.dart`'s own, since a caller commonly imports both.

import 'package:alix_mobile/sync_client.dart' show SyncWriter;

/// Mirrors `PairedConflict`: a push conflict names the desktop's current
/// document (the write was refused); a pull conflict names the desktop's
/// incoming document that was kept aside as `.pulled` instead of landing,
/// because the phone had unpushed changes.
sealed class PairedConflict {
  const PairedConflict();

  /// The desktop-side writer either case names, for the conflict sheet's
  /// "last written by" wording.
  SyncWriter? get desktopSideWriter;
}

class PairedConflictPush extends PairedConflict {
  const PairedConflictPush({
    this.desktopRevision,
    this.pulledRevision,
    this.desktopWriter,
  });

  final int? desktopRevision;
  final int? pulledRevision;
  final SyncWriter? desktopWriter;

  @override
  SyncWriter? get desktopSideWriter => desktopWriter;
}

class PairedConflictPull extends PairedConflict {
  const PairedConflictPull({this.pulledRevision, this.pulledWriter});

  final int? pulledRevision;
  final SyncWriter? pulledWriter;

  @override
  SyncWriter? get desktopSideWriter => pulledWriter;
}

/// One entry-scoped deck's paired state (mirrors `PairedDeckState`). [path]
/// is root-relative to the entry (a workspace member's `decks/cells.md`, or
/// a loose deck's own flattened file name).
class SyncDeckState {
  const SyncDeckState({
    required this.deckId,
    required this.path,
    required this.unpushed,
    this.conflict,
  });

  final String deckId;
  final String path;
  final bool unpushed;
  final PairedConflict? conflict;
}

/// One pulled picker entry (mirrors `PairedEntryState`).
class SyncEntryState {
  const SyncEntryState({
    required this.entry,
    required this.kind,
    required this.decks,
  });

  final String entry;

  /// Exactly `workspace` or `deck`.
  final String kind;
  final List<SyncDeckState> decks;
}

/// One planned push (mirrors `PushPlanItem`). [document] is the absolute
/// path of the local progress document to read and send.
class SyncPushPlanItem {
  const SyncPushPlanItem({
    required this.deckId,
    required this.entry,
    required this.document,
    this.base,
    required this.phoneRevision,
  });

  final String deckId;
  final String entry;
  final String document;
  final int? base;
  final int phoneRevision;

  /// The `X-Alix-Pulled-Revision` header value docs/API.md section 4.12's
  /// grammar demands: `none`, or a canonical positive decimal.
  String get pulledRevisionHeader => base == null ? 'none' : base.toString();
}

/// Mirrors `PushOutcomeDto`: what `SyncPort.recordPush` is told after a
/// push attempt, so the lib can advance or mark the local push state.
sealed class SyncPushOutcome {
  const SyncPushOutcome();
}

class SyncPushAcceptedOutcome extends SyncPushOutcome {
  const SyncPushAcceptedOutcome(this.revision);

  final int revision;
}

class SyncPushConflictOutcome extends SyncPushOutcome {
  const SyncPushConflictOutcome({this.desktopRevision, this.desktopWriter});

  final int? desktopRevision;
  final SyncWriter? desktopWriter;
}

class SyncPushNotServedOutcome extends SyncPushOutcome {
  const SyncPushNotServedOutcome();
}

/// Mirrors `ResolutionDto`: what `paired_resolve_conflict` says to do next.
sealed class SyncResolution {
  const SyncResolution();
}

class SyncResolutionDone extends SyncResolution {
  const SyncResolutionDone();
}

class SyncResolutionPush extends SyncResolution {
  const SyncResolutionPush(this.item);

  final SyncPushPlanItem item;
}

class SyncResolutionPull extends SyncResolution {
  const SyncResolutionPull(this.entry);

  final String entry;
}

/// One entry's pull outcome (mirrors `PullReportDto`). `landed`/`kept`/
/// `conflicts` are deck ids; `phoneOnly`/`removed` are entry-relative paths.
class SyncPullReport {
  const SyncPullReport({
    required this.entry,
    required this.kind,
    required this.landed,
    required this.kept,
    required this.conflicts,
    required this.phoneOnly,
    required this.removed,
  });

  final String entry;
  final String kind;
  final List<String> landed;
  final List<String> kept;
  final List<String> conflicts;
  final List<String> phoneOnly;
  final List<String> removed;
}

/// Mirrors `RenamedEntry`.
class SyncRenamedEntry {
  const SyncRenamedEntry({required this.old, required this.new_});

  final String old;
  final String new_;
}

/// What one full `SyncController.cycle` produced. Every list holds plain,
/// already-human-readable lines; `lib/sync/sync_sheet.dart` shows only the
/// non-empty ones.
class SyncReport {
  const SyncReport({
    this.landed = const [],
    this.kept = const [],
    this.conflicts = const [],
    this.phoneOnly = const [],
    this.removed = const [],
    this.leftOut = const [],
    this.renamed = const [],
    this.orphaned = const [],
    this.refused = const [],
    this.error,
  });

  final List<String> landed;
  final List<String> kept;
  final List<String> conflicts;
  final List<String> phoneOnly;
  final List<String> removed;
  final List<String> leftOut;
  final List<String> renamed;
  final List<String> orphaned;
  final List<String> refused;
  final String? error;

  bool get isEmpty =>
      error == null &&
      landed.isEmpty &&
      kept.isEmpty &&
      conflicts.isEmpty &&
      phoneOnly.isEmpty &&
      removed.isEmpty &&
      leftOut.isEmpty &&
      renamed.isEmpty &&
      orphaned.isEmpty &&
      refused.isEmpty;

  /// The picker's one-line status once a cycle ends and its report is
  /// unread. Names the categories that actually happened with their
  /// counts; a cycle that changed nothing reads as "up to date".
  String summary() {
    if (error != null) return error!;
    final parts = <String>[
      if (landed.isNotEmpty) '${landed.length} landed',
      if (conflicts.isNotEmpty)
        '${conflicts.length} conflict${conflicts.length == 1 ? '' : 's'}',
      if (refused.isNotEmpty) '${refused.length} refused',
      if (orphaned.isNotEmpty) '${orphaned.length} orphaned',
    ];
    if (parts.isEmpty) return 'Synced: up to date';
    return 'Synced: ${parts.join(', ')}';
  }
}

/// One deck that currently needs a conflict choice, sourced live from
/// `SyncPort.pairedEntries()` rather than a past cycle's report: a sheet
/// asking "keep phone or take desktop" needs the actual [conflict] to word
/// its buttons, not just the report's display string.
class SyncPendingConflict {
  const SyncPendingConflict({
    required this.deckId,
    required this.entry,
    required this.path,
    required this.conflict,
  });

  final String deckId;
  final String entry;
  final String path;
  final PairedConflict conflict;
}

/// Resolves the deck id a local document path corresponds to, from the
/// paired entries the phone already knows about. Mirrors the on-disk layout
/// `PairedRoot::entry_root` builds (`src/paired.rs`): a workspace member
/// lives at `<rootDir>/<entry>/<deck.path>`; a loose deck's document lands
/// flattened at `<rootDir>/<deck.path>` directly. Returns null when [path]
/// does not sit under [rootDir], or matches no known deck.
String? deckIdForPath({
  required List<SyncEntryState> entries,
  required String rootDir,
  required String path,
}) {
  final normalizedRoot = rootDir.replaceAll('\\', '/');
  final normalizedPath = path.replaceAll('\\', '/');
  if (!normalizedPath.startsWith(normalizedRoot)) return null;
  var relative = normalizedPath.substring(normalizedRoot.length);
  if (relative.startsWith('/')) relative = relative.substring(1);
  for (final entry in entries) {
    for (final deck in entry.decks) {
      final expected = entry.kind == 'workspace'
          ? '${entry.entry}/${deck.path}'
          : deck.path;
      if (expected == relative) return deck.deckId;
    }
  }
  return null;
}

/// "Keep the phone's progress" button wording for the conflict choice
/// sheet: names what it discards, and who last wrote the desktop side when
/// known.
String conflictKeepPhoneLabel(PairedConflict conflict) {
  final writer = conflict.desktopSideWriter;
  if (writer == null) {
    return "Keep the phone's progress (discards the desktop's version)";
  }
  return "Keep the phone's progress (discards the desktop's, "
      'last written by ${writer.device} at ${_formatTime(writer.atMs)})';
}

/// "Take the desktop's" button wording. The bridge carries no review count
/// for the phone side, so this names what is discarded without inventing a
/// number.
const String conflictTakeDesktopLabel =
    "Take the desktop's (discards the phone's progress since the last sync)";

String _formatTime(int atMs) {
  final local = DateTime.fromMillisecondsSinceEpoch(atMs).toLocal();
  final hh = local.hour.toString().padLeft(2, '0');
  final mm = local.minute.toString().padLeft(2, '0');
  return '$hh:$mm';
}
