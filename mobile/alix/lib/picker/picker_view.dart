import 'package:flutter/material.dart';

import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/picker_widgets.dart';
import 'package:alix_mobile/theme.dart';

class PickerView extends StatelessWidget {
  const PickerView({
    super.key,
    required this.entries,
    required this.conflicts,
    required this.conflictsDismissed,
    required this.deadline,
    required this.isRoot,
    required this.isMasteredView,
    required this.leading,
    this.title,
    this.staleDecksDir,
    required this.onOpenEntry,
    required this.onLongPressEntry,
    required this.onOpenMastered,
    required this.onAddTutorial,
    required this.onDismissConflicts,
  });

  final List<PickerEntry> entries;
  final List<String> conflicts;
  final bool conflictsDismissed;
  final PickerDeadline? deadline;
  final bool isRoot;
  final bool isMasteredView;
  final Widget leading;
  final String? title;
  final String? staleDecksDir;
  final ValueChanged<PickerEntry> onOpenEntry;
  final ValueChanged<PickerEntry> onLongPressEntry;
  final ValueChanged<List<PickerEntry>> onOpenMastered;
  final VoidCallback onAddTutorial;
  final VoidCallback onDismissConflicts;

  @override
  Widget build(BuildContext context) {
    final splitMastered = isRoot && !isMasteredView;
    final active = splitMastered
        ? entries.where((entry) => !entry.mastered).toList()
        : entries;
    final mastered = splitMastered
        ? entries.where((entry) => entry.mastered).toList()
        : const <PickerEntry>[];
    return Scaffold(
      appBar: alixAppBar(context, leading: leading),
      body: Column(
        children: [
          if (staleDecksDir != null)
            PickerNotice(
              text:
                  'Shared folder $staleDecksDir is unavailable; using app '
                  'storage for now.',
            ),
          if (conflicts.isNotEmpty && !conflictsDismissed)
            PickerConflictBanner(
              count: conflicts.length,
              onDismiss: onDismissConflicts,
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
                if (entries.isEmpty)
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
              ],
            ),
          ),
        ],
      ),
    );
  }
}
