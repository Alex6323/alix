import 'package:flutter/material.dart';

import 'package:alix_mobile/sync/sync_models.dart';

/// The sync report sheet: the non-empty categories from the last cycle,
/// and, for every deck that currently needs a conflict choice, two buttons
/// naming what each discards.
class SyncReportSheet extends StatelessWidget {
  const SyncReportSheet({
    super.key,
    required this.report,
    required this.conflicts,
    required this.onResolve,
    required this.onRemoveOrphan,
  });

  final SyncReport? report;
  final List<SyncPendingConflict> conflicts;
  final void Function(String deckId, bool keepPhone) onResolve;
  final ValueChanged<String> onRemoveOrphan;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final report = this.report;
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
              for (final conflict in conflicts)
                _ConflictChoice(conflict: conflict, onResolve: onResolve),
              if (report != null) ...[
                _Section(title: 'Landed', lines: report.landed),
                _Section(title: 'Kept (unpushed)', lines: report.kept),
                _Section(title: 'Phone-only', lines: report.phoneOnly),
                _Section(title: 'Removed', lines: report.removed),
                _Section(title: 'Renamed', lines: report.renamed),
                _Section(title: 'Left out', lines: report.leftOut),
                _Section(title: 'Refused', lines: report.refused),
                if (report.orphaned.isNotEmpty) ...[
                  Text('Orphaned', style: theme.textTheme.labelMedium),
                  for (final entry in report.orphaned)
                    _OrphanRow(entry: entry, onRemove: onRemoveOrphan),
                ],
              ],
              if (report == null && conflicts.isEmpty)
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
  const _OrphanRow({required this.entry, required this.onRemove});

  final String entry;
  final ValueChanged<String> onRemove;

  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        Expanded(
          child: Text(entry, maxLines: 1, overflow: TextOverflow.ellipsis),
        ),
        TextButton(
          onPressed: () => onRemove(entry),
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
