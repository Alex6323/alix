import 'dart:io';

import 'package:flutter/foundation.dart' show listEquals;
import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';

import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/tree_guides.dart';
import 'package:alix_mobile/shared/tier_colors.dart';
import 'package:alix_mobile/sync/sync_models.dart' show humanBytes;
import 'package:alix_mobile/sync_client.dart' show SyncEntry;
import 'package:alix_mobile/theme.dart';

TextStyle _ledeStyle(BuildContext context, Color? color) {
  return TextStyle(
    fontFamily: 'IBM Plex Mono',
    color: color ?? Theme.of(context).alix.bolt,
    fontSize: 12,
    letterSpacing: 2.2,
    fontWeight: FontWeight.w500,
  );
}

class PickerLede extends StatelessWidget {
  const PickerLede({super.key, required this.text, this.color});

  final String text;

  /// Overrides the default accent color, e.g. a subdued group label for a
  /// paired desktop's section. Defaults to `tokens.bolt`.
  final Color? color;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(2, 0, 2, 16),
      child: Text(
        text.toUpperCase(),
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: _ledeStyle(context, color),
      ),
    );
  }
}

/// The open folder's name in the app bar's title slot, where the root screen
/// shows the wordmark.
class PickerTitle extends StatelessWidget {
  const PickerTitle({super.key, required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    return Text(
      text.toUpperCase(),
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: _ledeStyle(context, null),
    );
  }
}

/// The paired desktop's group heading: what this phone is paired with, and
/// the one Sync that covers every entry beneath it.
class PickerPairedHeading extends StatelessWidget {
  const PickerPairedHeading({
    super.key,
    required this.label,
    this.onSyncAll,
    this.busy = false,
  });

  final String label;
  final VoidCallback? onSyncAll;
  final bool busy;

  @override
  Widget build(BuildContext context) {
    final tokens = Theme.of(context).alix;
    return Padding(
      padding: const EdgeInsets.fromLTRB(2, 0, 2, 10),
      child: Row(
        children: [
          Expanded(
            child: Text(
              label.toUpperCase(),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: _ledeStyle(context, tokens.dim),
            ),
          ),
          if (onSyncAll != null)
            TextButton(
              onPressed: busy ? null : onSyncAll,
              style: TextButton.styleFrom(
                minimumSize: Size.zero,
                padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
                tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                foregroundColor: tokens.bolt,
                textStyle: const TextStyle(
                  fontFamily: 'IBM Plex Mono',
                  fontSize: 12,
                  letterSpacing: 1.6,
                  fontWeight: FontWeight.w500,
                ),
              ),
              child: const Text('SYNC'),
            ),
        ],
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
    this.strip,
    this.flat = false,
  });

  final PickerEntry entry;
  final VoidCallback onTap;
  final VoidCallback? onLongPress;
  final PickerStrip? strip;

  final bool flat;

  @override
  Widget build(BuildContext context) {
    if (entry.tree.isNotEmpty && !flat) {
      return _PickerMemberRow(
        entry: entry,
        strip: strip,
        onTap: onTap,
        onLongPress: onLongPress,
      );
    }
    final hasAvatar = entry.isWorkspace || entry.icon != null;
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
              padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 11),
              decoration: BoxDecoration(
                border: Border.all(color: tokens.line),
                borderRadius: BorderRadius.circular(11),
              ),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      if (hasAvatar) ...[
                        _PickerAvatar(entry: entry),
                        const SizedBox(width: 12),
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
                    ],
                  ),
                  if (!entry.isWorkspace)
                    Padding(
                      padding: EdgeInsets.only(
                        top: 4,
                        left: hasAvatar ? _PickerAvatar.size + 12 : 0,
                      ),
                      child: PickerTierStrip(strip: strip),
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

class _PickerMemberRow extends StatelessWidget {
  const _PickerMemberRow({
    required this.entry,
    required this.strip,
    required this.onTap,
    this.onLongPress,
  });

  final PickerEntry entry;
  final PickerStrip? strip;
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
                      child: Column(
                        mainAxisSize: MainAxisSize.min,
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Row(
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
                          Padding(
                            padding: const EdgeInsets.only(top: 4),
                            child: PickerTierStrip(strip: strip),
                          ),
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

/// A desktop entry this phone has never pulled (`SyncController.
/// availableEntries`): name and size only, subdued, no `⋮` menu and no
/// review action. Tapping it pulls the entry for the first time; once the
/// pull lands the entry gains a manifest and this row is gone next build.
class PickerAvailableEntryRow extends StatelessWidget {
  const PickerAvailableEntryRow({
    super.key,
    required this.entry,
    required this.onTap,
  });

  final SyncEntry entry;
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
              border: Border.all(color: tokens.line),
              borderRadius: BorderRadius.circular(11),
            ),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    entry.name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.titleMedium?.copyWith(
                      color: tokens.dim,
                    ),
                  ),
                ),
                const SizedBox(width: 12),
                Text(
                  humanBytes(entry.unpackedBytes),
                  style: TextStyle(
                    fontFamily: 'IBM Plex Mono',
                    fontSize: 11,
                    color: tokens.dim,
                  ),
                ),
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
              ? 'No decks here yet. Put Markdown (.md) decks in this folder.'
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

/// What the depth sheet launches: a depth and the two modifiers above it.
typedef PickerLaunch = ({
  PickerDepth depth,
  bool cram,
  bool skipIntroduction,
});

class PickerDepthSheet extends StatefulWidget {
  const PickerDepthSheet({
    super.key,
    required this.canRecognize,
    required this.onChoose,
    this.onWalk,
  });

  final bool canRecognize;
  final ValueChanged<PickerLaunch> onChoose;
  final VoidCallback? onWalk;

  @override
  State<PickerDepthSheet> createState() => _PickerDepthSheetState();
}

class _PickerDepthSheetState extends State<PickerDepthSheet> {
  bool _cram = false;
  bool _skipIntroduction = false;

  @override
  Widget build(BuildContext context) {
    final canRecognize = widget.canRecognize;
    // A modal sheet caps its height; five rows exceed that cap on a short
    // window, so the sheet scrolls instead of clipping its last launch.
    return SafeArea(
      child: SingleChildScrollView(
        padding: const EdgeInsets.fromLTRB(16, 8, 16, 10),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            SwitchListTile(
              value: _cram,
              onChanged: (on) => setState(() => _cram = on),
              title: const Text('Cram'),
              subtitle: Text(
                _cram
                    ? 'now pick a depth to start'
                    : "also cards that aren't due yet",
              ),
            ),
            SwitchListTile(
              value: _skipIntroduction,
              onChanged: (on) => setState(() => _skipIntroduction = on),
              title: const Text('Skip introduction'),
              subtitle: const Text('new cards are graded at first sight'),
            ),
            const SizedBox(height: 8),
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
              _PickerLaunchRow(
                label: label,
                hint: hint,
                onTap: canRecognize || depth != PickerDepth.recognize
                    ? () => widget.onChoose((
                        depth: depth,
                        cram: _cram,
                        skipIntroduction: _skipIntroduction,
                      ))
                    : null,
              ),
            if (widget.onWalk case final onWalk?) ...[
              const SizedBox(height: 8),
              _PickerLaunchRow(
                label: 'Walk',
                hint: 'read every card once, no grading',
                onTap: onWalk,
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _PickerLaunchRow extends StatelessWidget {
  const _PickerLaunchRow({
    required this.label,
    required this.hint,
    required this.onTap,
  });

  final String label;
  final String hint;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    return Padding(
      padding: const EdgeInsets.only(bottom: 6),
      child: Opacity(
        opacity: onTap == null ? 0.5 : 1,
        child: Material(
          color: Colors.transparent,
          child: InkWell(
            borderRadius: BorderRadius.circular(11),
            onTap: onTap,
            child: Container(
              constraints: const BoxConstraints(minHeight: 54),
              padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 11),
              decoration: BoxDecoration(
                border: Border.all(color: tokens.line),
                borderRadius: BorderRadius.circular(11),
              ),
              child: Row(
                children: [
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          label,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: theme.textTheme.titleMedium?.copyWith(
                            fontWeight: FontWeight.w600,
                          ),
                        ),
                        Text(
                          hint,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(fontSize: 12, color: tokens.dim),
                        ),
                      ],
                    ),
                  ),
                  Icon(Icons.chevron_right, size: 22, color: tokens.dim),
                ],
              ),
            ),
          ),
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
  return const [];
}

/// A declared icon; failing that, for a workspace, a disc carrying its
/// initial.
class _PickerAvatar extends StatelessWidget {
  const _PickerAvatar({required this.entry});

  final PickerEntry entry;

  static const size = 32.0;

  @override
  Widget build(BuildContext context) {
    final tokens = Theme.of(context).alix;
    final path = entry.icon;
    if (path == null) {
      return Container(
        width: size,
        height: size,
        alignment: Alignment.center,
        decoration: BoxDecoration(color: tokens.line, shape: BoxShape.circle),
        child: Text(
          _initial(entry.title),
          style: TextStyle(
            fontFamily: 'IBM Plex Mono',
            fontSize: 15,
            fontWeight: FontWeight.w600,
            color: tokens.dim,
          ),
        ),
      );
    }
    return ClipOval(
      child: SizedBox(
        width: size,
        height: size,
        child: path.toLowerCase().endsWith('.svg')
            ? SvgPicture.file(
                File(path),
                fit: BoxFit.cover,
                colorFilter: ColorFilter.mode(tokens.dim, BlendMode.srcIn),
              )
            : Image.file(File(path), fit: BoxFit.cover),
      ),
    );
  }
}

String _initial(String title) {
  final trimmed = title.trim();
  if (trimmed.isEmpty) return '·';
  return String.fromCharCode(trimmed.runes.first).toUpperCase();
}

class PickerTierStrip extends StatelessWidget {
  const PickerTierStrip({super.key, required this.strip});

  final PickerStrip? strip;

  static const double height = 14;
  static const double barHeight = 3;
  static const double countWidth = 32;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    final strip = this.strip;
    return SizedBox(
      height: height,
      child: Row(
        children: [
          Expanded(
            child: SizedBox(
              key: const ValueKey('picker-strip-bar'),
              height: barHeight,
              child: strip == null
                  ? null
                  : CustomPaint(
                      painter: _TierStripPainter([
                        for (final tier in strip.tiers)
                          tierColor(
                            tier,
                            ink: theme.colorScheme.onSurface,
                            tokens: tokens,
                          ),
                      ]),
                    ),
            ),
          ),
          const SizedBox(width: 8),
          SizedBox(
            key: const ValueKey('picker-strip-count'),
            width: countWidth,
            child: strip == null
                ? null
                : Text(
                    '${strip.cardCount}',
                    maxLines: 1,
                    softWrap: false,
                    overflow: TextOverflow.clip,
                    textAlign: TextAlign.right,
                    style: TextStyle(
                      fontFamily: 'IBM Plex Mono',
                      fontSize: 11,
                      height: 1.2,
                      color: tokens.dim,
                    ),
                  ),
          ),
        ],
      ),
    );
  }
}

class _TierStripPainter extends CustomPainter {
  _TierStripPainter(this.colors);

  final List<Color> colors;

  @override
  void paint(Canvas canvas, Size size) {
    if (colors.isEmpty) return;
    final step = size.width / colors.length;
    final gap = step >= 3 ? 1.0 : 0.0;
    final paint = Paint();
    for (final (index, color) in colors.indexed) {
      paint.color = color;
      canvas.drawRect(
        Rect.fromLTWH(index * step, 0, step - gap, size.height),
        paint,
      );
    }
  }

  @override
  bool shouldRepaint(_TierStripPainter old) => !listEquals(old.colors, colors);
}

class PickerSearchField extends StatefulWidget {
  const PickerSearchField({
    super.key,
    required this.onChanged,
    required this.onClose,
  });

  final ValueChanged<String> onChanged;
  final VoidCallback onClose;

  @override
  State<PickerSearchField> createState() => _PickerSearchFieldState();
}

class _PickerSearchFieldState extends State<PickerSearchField> {
  final _text = TextEditingController();

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final tokens = Theme.of(context).alix;
    return Padding(
      padding: const EdgeInsets.only(bottom: 10),
      child: TextField(
        key: const ValueKey('picker-search-field'),
        controller: _text,
        autofocus: true,
        maxLines: 1,
        textInputAction: TextInputAction.search,
        onChanged: widget.onChanged,
        decoration: InputDecoration(
          isDense: true,
          hintText: 'Search decks',
          prefixIcon: Icon(Icons.search, size: 20, color: tokens.dim),
          suffixIcon: IconButton(
            icon: Icon(Icons.close, size: 20, color: tokens.dim),
            tooltip: 'Close search',
            onPressed: widget.onClose,
          ),
          border: OutlineInputBorder(
            borderRadius: BorderRadius.circular(11),
            borderSide: BorderSide(color: tokens.line),
          ),
          enabledBorder: OutlineInputBorder(
            borderRadius: BorderRadius.circular(11),
            borderSide: BorderSide(color: tokens.line),
          ),
        ),
      ),
    );
  }
}

class PickerSearchEmpty extends StatelessWidget {
  const PickerSearchEmpty({super.key});

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Text(
      'No decks match.',
      style: theme.textTheme.bodyMedium?.copyWith(color: theme.alix.dim),
    );
  }
}
