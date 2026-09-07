import 'package:flutter/material.dart';

import 'package:alix_mobile/sync/sync_models.dart';

/// Shows [conflict] alone, with no report section: the review summary's
/// post-push conflict, and a picker row blocked by a pending conflict, both
/// use this rather than the full [SyncReportSheet]. Pops itself once a
/// choice is made.
Future<void> showConflictChoiceSheet(
  BuildContext context, {
  required SyncPendingConflict conflict,
  required void Function(String deckId, bool keepPhone) onResolve,
}) {
  return showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    builder: (sheet) => SyncReportSheet(
      report: null,
      conflicts: [conflict],
      onResolve: (deckId, keepPhone) {
        onResolve(deckId, keepPhone);
        Navigator.of(sheet).pop();
      },
      onRemoveOrphan: (_) {},
    ),
  );
}

/// The sync report sheet: the non-empty categories from the last cycle,
/// and, for every deck that currently needs a conflict choice, two buttons
/// naming what each discards. [report] and [conflicts] are read once, when
/// the sheet opens; a resolve or an orphan removal updates this widget's
/// own display immediately (its row is gone), independent of whether the
/// caller ever hands down a fresher snapshot.
class SyncReportSheet extends StatefulWidget {
  const SyncReportSheet({
    super.key,
    required this.report,
    required this.conflicts,
    this.unpushedOrphans = const {},
    required this.onResolve,
    required this.onRemoveOrphan,
  });

  final SyncReport? report;
  final List<SyncPendingConflict> conflicts;

  /// Names of orphaned entries with unpushed local progress; the orphan
  /// row's confirmation names this when removing one.
  final Set<String> unpushedOrphans;
  final void Function(String deckId, bool keepPhone) onResolve;
  final ValueChanged<String> onRemoveOrphan;

  @override
  State<SyncReportSheet> createState() => _SyncReportSheetState();
}

class _SyncReportSheetState extends State<SyncReportSheet> {
  late List<SyncPendingConflict> _conflicts = List.of(widget.conflicts);
  late SyncReport? _report = widget.report;

  void _resolve(String deckId, bool keepPhone) {
    widget.onResolve(deckId, keepPhone);
    setState(() {
      _conflicts = [
        for (final conflict in _conflicts)
          if (conflict.deckId != deckId) conflict,
      ];
    });
  }

  void _removeOrphan(String entry) {
    widget.onRemoveOrphan(entry);
    setState(() {
      if (_report case final report?) _report = report.withoutOrphan(entry);
    });
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final report = _report;
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(24, 24, 24, 24),
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text('Sync', style: theme.textTheme.titleMedium),
              const SizedBox(height: 12),
              if (report?.error case final error?)
                Padding(
                  padding: const EdgeInsets.only(bottom: 12),
                  child: Text(
                    error,
                    style: TextStyle(color: theme.colorScheme.error),
                  ),
                ),
              for (final conflict in _conflicts)
                _ConflictChoice(conflict: conflict, onResolve: _resolve),
              if (report != null) ...[
                _Section(title: 'Landed', lines: report.landed),
                _Section(title: 'Kept (unpushed)', lines: report.kept),
                _Section(title: 'Phone-only', lines: report.phoneOnly),
                _Section(title: 'Removed', lines: report.removed),
                _Section(title: 'Renamed', lines: report.renamed),
                _Section(
                  title: 'Left out on the desktop',
                  lines: report.leftOut,
                ),
                _Section(title: 'Not on this phone', lines: report.notOnPhone),
                _Section(title: 'Refused', lines: report.refused),
                if (report.orphaned.isNotEmpty) ...[
                  Text('Orphaned', style: theme.textTheme.labelMedium),
                  for (final entry in report.orphaned)
                    _OrphanRow(
                      entry: entry,
                      unpushed: widget.unpushedOrphans.contains(entry),
                      onRemove: _removeOrphan,
                    ),
                ],
              ],
              if (report == null && _conflicts.isEmpty)
                Text(
                  'Nothing to report yet.',
                  style: theme.textTheme.bodySmall?.copyWith(
                    color: theme.colorScheme.onSurfaceVariant,
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

class _Section extends StatelessWidget {
  const _Section({required this.title, required this.lines});

  final String title;
  final List<String> lines;

  @override
  Widget build(BuildContext context) {
    if (lines.isEmpty) return const SizedBox.shrink();
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.only(bottom: 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(title, style: theme.textTheme.labelMedium),
          for (final line in lines)
            Text(line, maxLines: 1, overflow: TextOverflow.ellipsis),
        ],
      ),
    );
  }
}

class _OrphanRow extends StatelessWidget {
  const _OrphanRow({
    required this.entry,
    required this.unpushed,
    required this.onRemove,
  });

  final String entry;

  /// Whether any of [entry]'s decks carries unpushed local progress; named
  /// in the removal confirmation, since removal discards it.
  final bool unpushed;
  final ValueChanged<String> onRemove;

  Future<void> _confirm(BuildContext context) async {
    final message = unpushed
        ? 'Remove "$entry" from this phone? Its unpushed progress goes too.'
        : 'Remove "$entry" from this phone?';
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (dialogContext) => AlertDialog(
        title: const Text('Remove entry'),
        content: Text(message),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(dialogContext).pop(false),
            child: const Text('Cancel'),
          ),
          TextButton(
            onPressed: () => Navigator.of(dialogContext).pop(true),
            child: const Text('Remove'),
          ),
        ],
      ),
    );
    if (confirmed == true) onRemove(entry);
  }

  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        Expanded(
          child: Text(entry, maxLines: 1, overflow: TextOverflow.ellipsis),
        ),
        TextButton(
          onPressed: () => _confirm(context),
          child: const Text('Remove'),
        ),
      ],
    );
  }
}

class _ConflictChoice extends StatelessWidget {
  const _ConflictChoice({required this.conflict, required this.onResolve});

  final SyncPendingConflict conflict;
  final void Function(String deckId, bool keepPhone) onResolve;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final label = '${conflict.entry}/${conflict.path.split('/').last}';
    return Padding(
      padding: const EdgeInsets.only(bottom: 16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            label,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: theme.textTheme.titleSmall,
          ),
          const SizedBox(height: 8),
          OutlinedButton(
            onPressed: () => onResolve(conflict.deckId, true),
            child: Text(
              conflictKeepPhoneLabel(conflict.conflict),
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              textAlign: TextAlign.center,
            ),
          ),
          const SizedBox(height: 8),
          OutlinedButton(
            onPressed: () => onResolve(conflict.deckId, false),
            child: const Text(
              conflictTakeDesktopLabel,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              textAlign: TextAlign.center,
            ),
          ),
        ],
      ),
    );
  }
}
