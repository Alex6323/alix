import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';

import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/tree_guides.dart';
import 'package:alix_mobile/theme.dart';

class PickerLede extends StatelessWidget {
  const PickerLede({super.key, required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(2, 0, 2, 16),
      child: Text(
        text.toUpperCase(),
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: TextStyle(
          fontFamily: 'IBM Plex Mono',
          color: Theme.of(context).alix.bolt,
          fontSize: 12,
          letterSpacing: 2.2,
          fontWeight: FontWeight.w500,
        ),
      ),
    );
  }
}

class PickerDeadlineLede extends StatelessWidget {
  const PickerDeadlineLede({super.key, required this.deadline});

  final PickerDeadline deadline;

  @override
  Widget build(BuildContext context) {
    final when = deadline.daysLeft < 0
        ? 'was due ${deadline.date}'
        : deadline.date;
    return Padding(
      padding: const EdgeInsets.fromLTRB(2, 0, 2, 16),
      child: Text(
        '🎯 $when · ${deadline.ready}/${deadline.total} mastered',
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: TextStyle(
          fontFamily: 'IBM Plex Mono',
          fontSize: 12,
          color: pickerDeadlineTint(deadline, Theme.of(context).alix),
        ),
      ),
    );
  }
}

class PickerDeckRow extends StatelessWidget {
  const PickerDeckRow({
    super.key,
    required this.entry,
    required this.onTap,
    this.onLongPress,
  });

  final PickerEntry entry;
  final VoidCallback onTap;
  final VoidCallback? onLongPress;

  @override
  Widget build(BuildContext context) {
    if (entry.tree.isNotEmpty) {
      return _PickerMemberRow(
        entry: entry,
        onTap: onTap,
        onLongPress: onLongPress,
      );
    }
    final theme = Theme.of(context);
    final tokens = theme.alix;
    return Padding(
      padding: const EdgeInsets.only(bottom: 6),
      child: Opacity(
        opacity: entry.locked ? 0.5 : 1,
        child: Material(
          color: Colors.transparent,
          child: InkWell(
            borderRadius: BorderRadius.circular(11),
            onTap: onTap,
            onLongPress: onLongPress,
            child: Container(
              constraints: const BoxConstraints(minHeight: 54),
              padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
              decoration: BoxDecoration(
                border: Border.all(color: tokens.line),
                borderRadius: BorderRadius.circular(11),
              ),
              child: Row(
                children: [
                  if (entry.icon != null) ...[
                    _PickerEmblem(path: entry.icon!),
                    const SizedBox(width: 10),
                  ],
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          entry.title,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: theme.textTheme.titleMedium?.copyWith(
                            fontWeight: FontWeight.w600,
                          ),
                        ),
                        if (entry.deadline case final deadline?)
                          Text(
                            pickerDeadlineChipText(deadline),
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              fontFamily: 'IBM Plex Mono',
                              fontSize: 11,
                              color: pickerDeadlineTint(deadline, tokens),
                            ),
                          ),
                      ],
                    ),
                  ),
                  ...pickerTrailingMarker(theme, entry),
                  if (entry.isWorkspace) ...[
                    const SizedBox(width: 8),
                    Icon(Icons.chevron_right, size: 22, color: tokens.dim),
                  ],
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _PickerMemberRow extends StatelessWidget {
  const _PickerMemberRow({
    required this.entry,
    required this.onTap,
    this.onLongPress,
  });

  final PickerEntry entry;
  final VoidCallback onTap;
  final VoidCallback? onLongPress;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    return Opacity(
      opacity: entry.locked ? 0.5 : 1,
      child: Material(
        color: Colors.transparent,
        child: InkWell(
          onTap: onTap,
          onLongPress: onLongPress,
          child: ConstrainedBox(
            constraints: const BoxConstraints(minHeight: 46),
            child: IntrinsicHeight(
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Padding(
                    padding: const EdgeInsets.only(left: 8),
                    child: TreeGuides(tree: entry.tree, color: tokens.dim),
                  ),
                  Expanded(
                    child: Padding(
                      padding: const EdgeInsets.fromLTRB(8, 12, 16, 12),
                      child: Row(
                        children: [
                          Expanded(
                            child: Text(
                              entry.title,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: theme.textTheme.titleMedium?.copyWith(
                                fontWeight: FontWeight.w600,
                              ),
                            ),
                          ),
                          ...pickerTrailingMarker(theme, entry),
                        ],
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class PickerMasteredAffordance extends StatelessWidget {
  const PickerMasteredAffordance({
    super.key,
    required this.count,
    required this.onTap,
  });

  final int count;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    return Padding(
      padding: const EdgeInsets.only(bottom: 6),
      child: Material(
        color: Colors.transparent,
        child: InkWell(
          borderRadius: BorderRadius.circular(11),
          onTap: onTap,
          child: Container(
            constraints: const BoxConstraints(minHeight: 54),
            padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
            decoration: BoxDecoration(
              border: Border.all(color: tokens.good.withValues(alpha: 0.4)),
              borderRadius: BorderRadius.circular(11),
            ),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    'Mastered · $count',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.titleMedium?.copyWith(
                      fontWeight: FontWeight.w600,
                      color: tokens.good,
                    ),
                  ),
                ),
                Icon(Icons.chevron_right, size: 22, color: tokens.good),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class PickerEmptyHint extends StatelessWidget {
  const PickerEmptyHint({
    super.key,
    required this.atRoot,
    required this.onAddTutorial,
  });

  final bool atRoot;
  final VoidCallback onAddTutorial;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          atRoot
              ? 'No decks here yet. Put Markdown (.md) decks in this folder, or '
                    'choose a shared folder from Settings.'
              : 'no decks here',
          style: theme.textTheme.bodyMedium?.copyWith(color: theme.alix.dim),
        ),
        if (atRoot) ...[
          const SizedBox(height: 16),
          OutlinedButton.icon(
            onPressed: onAddTutorial,
            icon: const Icon(Icons.school_outlined),
            label: const Text('Add the tutorial deck'),
          ),
        ],
      ],
    );
  }
}

class PickerNotice extends StatelessWidget {
  const PickerNotice({super.key, required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
      child: Text(
        text,
        style: theme.textTheme.bodySmall?.copyWith(
          color: theme.colorScheme.onSurfaceVariant,
        ),
      ),
    );
  }
}

class PickerConflictBanner extends StatelessWidget {
  const PickerConflictBanner({
    super.key,
    required this.count,
    required this.onDismiss,
  });

  final int count;
  final VoidCallback onDismiss;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Container(
      margin: const EdgeInsets.fromLTRB(12, 8, 12, 0),
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
      decoration: BoxDecoration(
        color: theme.colorScheme.errorContainer,
        borderRadius: BorderRadius.circular(8),
      ),
      child: Row(
        children: [
          Expanded(
            child: Text(
              'A sync conflict file sits next to your progress ($count). '
              'Review on one device at a time and resolve it first; see the '
              'manual.',
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.colorScheme.onErrorContainer,
              ),
            ),
          ),
          IconButton(
            icon: const Icon(Icons.close, size: 18),
            onPressed: onDismiss,
          ),
        ],
      ),
    );
  }
}

class PickerSupportSheet extends StatelessWidget {
  const PickerSupportSheet({super.key});

  @override
  Widget build(BuildContext context) {
    final dimStyle = Theme.of(
      context,
    ).textTheme.bodySmall?.copyWith(color: Theme.of(context).alix.dim);
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              'Support alix',
              style: Theme.of(context).textTheme.titleMedium,
            ),
            const SizedBox(height: 12),
            Text(
              'Free and open source. Telling someone who studies is the best '
              'support.',
              style: dimStyle,
            ),
            const SizedBox(height: 8),
            SelectableText(
              'https://github.com/sponsors/Alex6323',
              style: dimStyle,
            ),
          ],
        ),
      ),
    );
  }
}

class PickerDeadlineSheet extends StatelessWidget {
  const PickerDeadlineSheet({
    super.key,
    required this.current,
    required this.onPick,
    required this.onClear,
  });

  final PickerDeadline? current;
  final VoidCallback onPick;
  final VoidCallback onClear;

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ListTile(
            leading: const SizedBox(width: 22, child: Text('🎯')),
            title: const Text('Ready by…'),
            subtitle: Text(
              current == null
                  ? 'set a target date for this workspace'
                  : 'currently ${current!.date}',
            ),
            onTap: onPick,
          ),
          if (current != null)
            ListTile(
              leading: const SizedBox(width: 22),
              title: const Text('Clear deadline'),
              onTap: onClear,
            ),
        ],
      ),
    );
  }
}

class PickerDepthSheet extends StatelessWidget {
  const PickerDepthSheet({
    super.key,
    this.selected,
    required this.canRecognize,
    required this.onChoose,
  });

  final PickerDepth? selected;
  final bool canRecognize;
  final ValueChanged<PickerDepth> onChoose;

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          for (final (depth, label, hint) in [
            (
              PickerDepth.recognize,
              'Recognize',
              canRecognize
                  ? 'pick the answer out of four'
                  : 'augment the deck to enable',
            ),
            (PickerDepth.recall, 'Recall', 'the everyday review'),
            (
              PickerDepth.reconstruct,
              'Reconstruct',
              'type or rebuild the answer',
            ),
          ])
            ListTile(
              enabled: canRecognize || depth != PickerDepth.recognize,
              leading: SizedBox(
                width: 22,
                child: depth == selected
                    ? Icon(
                        Icons.check,
                        size: 18,
                        color: Theme.of(context).alix.bolt,
                      )
                    : null,
              ),
              title: Text(label),
              subtitle: Text(hint),
              onTap: () => onChoose(depth),
            ),
        ],
      ),
    );
  }
}

class PickerFolderSheet extends StatelessWidget {
  const PickerFolderSheet({
    super.key,
    required this.root,
    required this.supported,
    required this.hasSharedDir,
    required this.onChoose,
    required this.onUseAppStorage,
  });

  final String root;
  final bool supported;
  final bool hasSharedDir;
  final VoidCallback onChoose;
  final VoidCallback onUseAppStorage;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text('Decks folder', style: theme.textTheme.titleMedium),
            const SizedBox(height: 8),
            Text(
              root,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.colorScheme.onSurfaceVariant,
                fontFamily: 'monospace',
              ),
            ),
            const SizedBox(height: 16),
            if (supported)
              FilledButton(
                onPressed: onChoose,
                child: const Text('Choose shared folder…'),
              )
            else
              Text(
                'Shared folders need Android 11 or newer.',
                style: theme.textTheme.bodySmall?.copyWith(
                  color: theme.colorScheme.onSurfaceVariant,
                ),
              ),
            if (hasSharedDir) ...[
              const SizedBox(height: 8),
              TextButton(
                onPressed: onUseAppStorage,
                child: const Text('Use app storage'),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class PickerThemeSheet extends StatelessWidget {
  const PickerThemeSheet({
    super.key,
    required this.current,
    required this.onChoose,
  });

  final String current;
  final ValueChanged<String> onChoose;

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      child: SizedBox(
        height: MediaQuery.of(context).size.height * 0.75,
        child: ListView(
          key: const ValueKey('theme-sheet-list'),
          padding: const EdgeInsets.symmetric(vertical: 8),
          children: [
            for (final mode in const [Brightness.dark, Brightness.light]) ...[
              _PickerThemeGroupLabel(
                label: mode == Brightness.dark ? 'Dark' : 'Light',
              ),
              for (final theme in alixThemes.where((item) => item.mode == mode))
                _PickerThemeTile(
                  theme: theme,
                  current: current,
                  onChoose: onChoose,
                ),
            ],
          ],
        ),
      ),
    );
  }
}

class _PickerThemeGroupLabel extends StatelessWidget {
  const _PickerThemeGroupLabel({required this.label});

  final String label;

  @override
  Widget build(BuildContext context) {
    final tokens = Theme.of(context).alix;
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 4),
      child: Text(
        label.toUpperCase(),
        style: TextStyle(
          fontFamily: 'IBM Plex Mono',
          color: tokens.bolt,
          fontSize: 12,
          letterSpacing: 2.2,
          fontWeight: FontWeight.w500,
        ),
      ),
    );
  }
}

class _PickerThemeTile extends StatelessWidget {
  const _PickerThemeTile({
    required this.theme,
    required this.current,
    required this.onChoose,
  });

  final AlixTheme theme;
  final String current;
  final ValueChanged<String> onChoose;

  @override
  Widget build(BuildContext context) {
    final tokens = Theme.of(context).alix;
    return ListTile(
      key: ValueKey('theme-tile-${theme.id}'),
      leading: _PickerThemeSwatch(theme: theme),
      title: Text(theme.name, maxLines: 1, overflow: TextOverflow.ellipsis),
      trailing: theme.id == current
          ? Icon(Icons.check, size: 18, color: tokens.bolt)
          : null,
      onTap: () => onChoose(theme.id),
    );
  }
}

class _PickerThemeSwatch extends StatelessWidget {
  const _PickerThemeSwatch({required this.theme});

  final AlixTheme theme;

  @override
  Widget build(BuildContext context) {
    final scheme = theme.data.colorScheme;
    final tokens = theme.data.alix;
    return Container(
      width: 36,
      height: 24,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: scheme.surface,
        borderRadius: BorderRadius.circular(4),
        border: Border.all(color: tokens.line),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          _PickerThemeDot(color: tokens.bolt),
          const SizedBox(width: 4),
          _PickerThemeDot(color: tokens.good),
        ],
      ),
    );
  }
}

class _PickerThemeDot extends StatelessWidget {
  const _PickerThemeDot({required this.color});

  final Color color;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: 8,
      height: 8,
      decoration: BoxDecoration(color: color, shape: BoxShape.circle),
    );
  }
}

String pickerDeadlineChipText(PickerDeadline deadline) {
  if (deadline.daysLeft < 0) return '🎯 was due ${deadline.date}';
  final total = deadline.total == 0 ? 1 : deadline.total;
  return '🎯 ${deadline.date} · ${deadline.daysLeft}d · '
      '${(100 * deadline.ready / total).round()}%';
}

Color pickerDeadlineTint(PickerDeadline deadline, AlixTokens tokens) {
  if (deadline.daysLeft < 0) return tokens.warn;
  return deadline.daysLeft <= 7 ? tokens.bolt : tokens.dim;
}

List<Widget> pickerTrailingMarker(ThemeData theme, PickerEntry entry) {
  final tokens = theme.alix;
  if (entry.progressError) {
    return [
      const SizedBox(width: 12),
      Text(
        'error',
        style: theme.textTheme.labelSmall?.copyWith(
          color: tokens.again,
          fontFamily: 'monospace',
          letterSpacing: 1.2,
        ),
      ),
    ];
  }
  if (entry.isTrace) {
    return [
      const SizedBox(width: 12),
      Text(
        'trace',
        style: theme.textTheme.labelSmall?.copyWith(
          color: tokens.faint,
          fontFamily: 'monospace',
          letterSpacing: 1.2,
        ),
      ),
    ];
  }
  if (entry.examDue) {
    return [
      const SizedBox(width: 12),
      Text(
        'exam',
        style: theme.textTheme.labelSmall?.copyWith(
          color: tokens.warn,
          fontFamily: 'monospace',
          letterSpacing: 1.2,
        ),
      ),
    ];
  }
  if (entry.due) {
    return [
      const SizedBox(width: 12),
      Icon(Icons.circle, size: 8, color: tokens.bolt),
    ];
  }
  return const [];
}

class _PickerEmblem extends StatelessWidget {
  const _PickerEmblem({required this.path});

  final String path;

  @override
  Widget build(BuildContext context) {
    const size = 22.0;
    final color = Theme.of(context).alix.dim;
    if (path.toLowerCase().endsWith('.svg')) {
      return SvgPicture.file(
        File(path),
        width: size,
        height: size,
        colorFilter: ColorFilter.mode(color, BlendMode.srcIn),
      );
    }
    return ClipRRect(
      borderRadius: BorderRadius.circular(4),
      child: Image.file(
        File(path),
        width: size,
        height: size,
        fit: BoxFit.cover,
      ),
    );
  }
}
