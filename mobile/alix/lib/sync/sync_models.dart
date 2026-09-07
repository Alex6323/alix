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
    this.phoneSaves = 0,
    this.phoneAtMs,
    this.conflict,
  });

  final String deckId;
  final String path;
  final bool unpushed;

  /// The local document's revision past the last pushed phone revision:
  /// how many phone saves a take-desktop choice on this deck would discard.
  final int phoneSaves;

  /// The local document head's writer time, whenever it has one; null only
  /// when the phone holds no local copy to read a writer from.
  final int? phoneAtMs;
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
    this.conflictDeckIds = const [],
    this.phoneOnly = const [],
    this.removed = const [],
    this.leftOut = const [],
    this.notOnPhone = const [],
    this.renamed = const [],
    this.orphaned = const [],
    this.refused = const [],
    this.pushed = const [],
    this.error,
  });

  final List<String> landed;
  final List<String> kept;
  final List<String> conflicts;

  /// Parallel to [conflicts]: the deck id behind each row, index-aligned.
  /// A label is neither unique (two loose decks can share a `title:`) nor
  /// proof a resolve cleared that deck (a keep-phone retry can land a
  /// fresh conflict), so [SyncController.resolve] filters rows by this,
  /// never by matching the label string.
  final List<String> conflictDeckIds;
  final List<String> phoneOnly;
  final List<String> removed;

  /// Deck labels the desktop accepted this cycle (`SyncPushAccepted`). No
  /// "unchanged" counterpart: silence is the calm default when nothing
  /// changed, so a cycle that pushed nothing says nothing about it.
  final List<String> pushed;

  /// `'<entry>/<path>'` for every member the desktop could not load into an
  /// entry this phone has (or just pulled this cycle): `SyncEntry.leftOut`,
  /// carried through under the entry's own name.
  final List<String> leftOut;

  /// Names of desktop-served entries this phone has never pulled a
  /// manifest for (`SyncController.availableEntries`'s names as of this
  /// report).
  final List<String> notOnPhone;
  final List<String> renamed;
  final List<String> orphaned;
  final List<String> refused;
  final String? error;

  /// Whether the cycle changed or found nothing worth flagging as unread.
  /// [notOnPhone] is excluded on purpose: the picker's own never-pulled
  /// rows already say so, so it alone must not mark the status unread.
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
      refused.isEmpty &&
      pushed.isEmpty;

  /// The same report with [entry] dropped from [orphaned]; every other
  /// field is unchanged.
  SyncReport withoutOrphan(String entry) {
    if (!orphaned.contains(entry)) return this;
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
      orphaned: [for (final e in orphaned) if (e != entry) e],
      refused: refused,
      pushed: pushed,
      error: error,
    );
  }

  /// The same report with every conflict row whose deck id is not in
  /// [deckIds] dropped; every other field is unchanged.
  /// [SyncController.resolve] calls this with the refreshed
  /// `pendingConflicts`' deck ids after a resolve, so a duplicate title
  /// (two rows, same label, different deck) or a fresh conflict on a
  /// keep-phone retry survives, instead of vanishing because its label
  /// happened to match the one just resolved. A row with no recorded deck
  /// id (an older report shape) is dropped, since its identity cannot be
  /// checked.
  SyncReport keepingConflictsIn(Set<String> deckIds) {
    final keptLabels = <String>[];
    final keptIds = <String>[];
    for (var i = 0; i < conflicts.length; i++) {
      final id = i < conflictDeckIds.length ? conflictDeckIds[i] : null;
      if (id != null && deckIds.contains(id)) {
        keptLabels.add(conflicts[i]);
        keptIds.add(id);
      }
    }
    if (keptLabels.length == conflicts.length) return this;
    return SyncReport(
      landed: landed,
      kept: kept,
      conflicts: keptLabels,
      conflictDeckIds: keptIds,
      phoneOnly: phoneOnly,
      removed: removed,
      leftOut: leftOut,
      notOnPhone: notOnPhone,
      renamed: renamed,
      orphaned: orphaned,
      refused: refused,
      pushed: pushed,
      error: error,
    );
  }

  /// The picker's one-line status once a cycle ends and its report is
  /// unread. Names the categories that actually happened with their
  /// counts; a cycle that changed nothing reads as "up to date". No
  /// "unchanged" part: silence is the calm default, not a category.
  String summary() {
    if (error != null) return error!;
    final parts = <String>[
      if (landed.isNotEmpty) '${landed.length} landed',
      if (pushed.isNotEmpty) '${pushed.length} pushed',
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
    required this.label,
    required this.conflict,
    this.phoneSaves = 0,
    this.phoneAtMs,
  });

  final String deckId;

  /// A workspace member's `<entry>/<basename>`, or a loose deck's title
  /// (falling back to the entry name); never `<entry>/<entry>`.
  final String label;

  final PairedConflict conflict;

  /// Mirrors the conflicted deck's [SyncDeckState.phoneSaves] as of the
  /// scan that found this conflict, for [conflictTakeDesktopWording].
  final int phoneSaves;

  /// Mirrors the conflicted deck's [SyncDeckState.phoneAtMs].
  final int? phoneAtMs;
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

/// A conflict choice row's short title, plus a subtitle naming what
/// choosing it discards.
typedef ConflictChoiceWording = ({String title, String subtitle});

/// "Keep the phone's progress": names what it discards, and who last wrote
/// the desktop side when known.
ConflictChoiceWording conflictKeepPhoneWording(PairedConflict conflict) {
  final writer = conflict.desktopSideWriter;
  final subtitle = writer == null
      ? "discards the desktop's version"
      : "discards the desktop's, last written by ${writer.device} at "
            '${_formatTime(writer.atMs)}';
  return (title: "Keep the phone's progress", subtitle: subtitle);
}

/// "Take the desktop's": names the phone saves this choice discards, with
/// the last save's time when the local document carries a writer. A
/// conflict with nothing unsynced (`phoneSaves == 0`) has nothing to name.
ConflictChoiceWording conflictTakeDesktopWording(
  SyncPendingConflict conflict,
) {
  final saves = conflict.phoneSaves;
  if (saves <= 0) {
    return (
      title: "Take the desktop's",
      subtitle: 'nothing to discard on the phone',
    );
  }
  final noun = saves == 1 ? 'phone save' : 'phone saves';
  final atMs = conflict.phoneAtMs;
  final subtitle = atMs == null
      ? 'discards $saves $noun'
      : 'discards $saves $noun, last at ${_formatTime(atMs)}';
  return (title: "Take the desktop's", subtitle: subtitle);
}

// `YYYY-MM-DD HH:MM`, local time: date included, since a stale conflict can
// be days old.
String _formatTime(int atMs) {
  final local = DateTime.fromMillisecondsSinceEpoch(atMs).toLocal();
  final yyyy = local.year.toString().padLeft(4, '0');
  final month = local.month.toString().padLeft(2, '0');
  final day = local.day.toString().padLeft(2, '0');
  final hh = local.hour.toString().padLeft(2, '0');
  final mm = local.minute.toString().padLeft(2, '0');
  return '$yyyy-$month-$day $hh:$mm';
}

/// A byte count in the coarsest unit that keeps one decimal of precision,
/// shared by the free-space refusal line and the picker's never-pulled
/// entry rows.
String humanBytes(int bytes) {
  if (bytes >= 1024 * 1024) {
    return '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MB';
  }
  if (bytes >= 1024) return '${(bytes / 1024).toStringAsFixed(1)} KB';
  return '$bytes bytes';
}
