import 'package:flutter/material.dart';

import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/picker_widgets.dart';
import 'package:alix_mobile/sync_client.dart' show SyncEntry;
import 'package:alix_mobile/theme.dart';

class PickerView extends StatelessWidget {
  const PickerView({
    super.key,
    required this.entries,
    this.isLoading = false,
    required this.deadline,
    required this.isRoot,
    required this.isMasteredView,
    required this.leading,
    this.title,
    required this.onOpenEntry,
    required this.onLongPressEntry,
    required this.onOpenMastered,
    required this.onAddTutorial,
    this.syncStatus,
    this.onOpenSyncReport,
    this.pairedLabel,
    this.pairedEntries = const [],
    this.onOpenPairedEntry,
    this.onLongPressPairedEntry,
    this.onSyncEntry,
    this.availableEntries = const [],
    this.onPullAvailable,
  });

  final List<PickerEntry> entries;
  final bool isLoading;
  final PickerDeadline? deadline;
  final bool isRoot;
  final bool isMasteredView;
  final Widget leading;
  final String? title;
  final ValueChanged<PickerEntry> onOpenEntry;
  final ValueChanged<PickerEntry> onLongPressEntry;
  final ValueChanged<List<PickerEntry>> onOpenMastered;
  final VoidCallback onAddTutorial;

  /// One line shown above the list while a sync cycle runs or its report
  /// is unread; tapping it opens the report.
  final String? syncStatus;
  final VoidCallback? onOpenSyncReport;

  /// The active paired desktop's label ("host:port"), shown as a subdued
  /// group heading above [pairedEntries] and [availableEntries]. Null
  /// unless a pairing is active and this is the root screen.
  final String? pairedLabel;

  /// The paired desktop's own pulled top-level entries, listed below
  /// [entries] under [pairedLabel] rather than merged into them.
  final List<PickerEntry> pairedEntries;
  final ValueChanged<PickerEntry>? onOpenPairedEntry;
  final ValueChanged<PickerEntry>? onLongPressPairedEntry;

  /// Adds "Sync" to a paired entry row's overflow menu; never shown on a
  /// phone-own row.
  final ValueChanged<PickerEntry>? onSyncEntry;

  /// Desktop entries this phone has never pulled, shown under
  /// [pairedLabel] below [pairedEntries].
  final List<SyncEntry> availableEntries;
  final ValueChanged<String>? onPullAvailable;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final splitMastered = isRoot && !isMasteredView;
    final active = splitMastered
        ? entries.where((entry) => !entry.mastered).toList()
        : entries;
    final mastered = splitMastered
        ? entries.where((entry) => entry.mastered).toList()
        : const <PickerEntry>[];
    final showPairedSection =
        pairedLabel != null &&
        (pairedEntries.isNotEmpty || availableEntries.isNotEmpty);
    return Scaffold(
      appBar: alixAppBar(context, leading: leading),
      body: Column(
        children: [
          if (syncStatus case final status?)
            InkWell(
              key: const Key('sync-status'),
              onTap: onOpenSyncReport,
              child: Padding(
                padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
                child: Text(
                  status,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(fontSize: 12, color: theme.alix.dim),
                ),
              ),
            ),
          Expanded(
            child: ListView(
              padding: const EdgeInsets.fromLTRB(16, 16, 16, 24),
              children: [
                if (isMasteredView)
                  const PickerLede(text: 'Mastered 🎉')
                else if (!isRoot && title != null) ...[
                  PickerLede(text: title!),
                  if (deadline case final value?)
                    PickerDeadlineLede(deadline: value),
                ],
                if (isLoading)
                  const SizedBox.shrink()
                else if (entries.isEmpty)
                  PickerEmptyHint(
                    atRoot: isRoot && !isMasteredView,
                    onAddTutorial: onAddTutorial,
                  )
                else ...[
                  for (final entry in active)
                    PickerDeckRow(
                      entry: entry,
                      onTap: () => onOpenEntry(entry),
                      onLongPress:
                          (!entry.isWorkspace && !entry.isTrace) ||
                              (entry.tree.isNotEmpty && !entry.isTrace) ||
                              entry.isWorkspace
                          ? () => onLongPressEntry(entry)
                          : null,
                    ),
                  if (mastered.isNotEmpty)
                    PickerMasteredAffordance(
                      count: mastered.length,
                      onTap: () => onOpenMastered(mastered),
                    ),
                ],
                if (showPairedSection) ...[
                  const SizedBox(height: 8),
                  PickerLede(text: pairedLabel!, color: theme.alix.dim),
                  for (final entry in pairedEntries)
                    PickerDeckRow(
                      entry: entry,
                      onTap: () => onOpenPairedEntry?.call(entry),
                      onLongPress:
                          (!entry.isWorkspace && !entry.isTrace) ||
                              (entry.tree.isNotEmpty && !entry.isTrace) ||
                              entry.isWorkspace
                          ? () => onLongPressPairedEntry?.call(entry)
                          : null,
                      onSync: onSyncEntry == null
                          ? null
                          : () => onSyncEntry!(entry),
                    ),
                  for (final entry in availableEntries)
                    PickerAvailableEntryRow(
                      entry: entry,
                      onTap: () => onPullAvailable?.call(entry.name),
                    ),
                ],
              ],
            ),
          ),
        ],
      ),
    );
  }
}
