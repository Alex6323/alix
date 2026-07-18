import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:path_provider/path_provider.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/folder_browser.dart';
import 'package:alix_mobile/pairing_sheet.dart';
import 'package:alix_mobile/platform_access.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/src/rust/api/generate.dart';
import 'package:alix_mobile/src/rust/api/listing.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/api/simple.dart';
import 'package:alix_mobile/walk_screen.dart';

/// One list screen serving both levels: the decks root, and (with [dir]) a
/// drilled-into workspace or deck folder. The root level also owns the
/// rare controls (decks folder, About) behind a single overflow menu.
class PickerScreen extends StatefulWidget {
  const PickerScreen({
    super.key,
    required this.root,
    this.dir,
    this.title,
    this.device,
    this.sharedDir,
    this.staleDecksDir,
    this.access,
    this.onSetDecksDir,
    this.currentThemeId,
    this.onSetTheme,
    this.supportDir,
    this.buildClient,
    this.generatePollInterval,
  }) : masteredEntries = null;

  /// The Mastered window: this same screen with a fixed, pre-filtered entry
  /// list (no bridge listing call, no root chrome) so mastered decks stay
  /// openable to re-review via the ordinary row/tap path.
  const PickerScreen.mastered({
    super.key,
    required this.root,
    required List<DeckEntry> entries,
    this.device,
  })  : masteredEntries = entries,
        dir = null,
        title = null,
        sharedDir = null,
        staleDecksDir = null,
        access = null,
        onSetDecksDir = null,
        currentThemeId = null,
        onSetTheme = null,
        supportDir = null,
        buildClient = null,
        generatePollInterval = null;

  final String root;
  final String? dir;
  final String? title;

  /// This install's label for the store's last-writer marker.
  final String? device;

  /// The persisted shared-folder setting, if any (shown in the folder
  /// sheet; enables the revert action).
  final String? sharedDir;

  /// Set when the shared folder was unusable this launch.
  final String? staleDecksDir;

  /// Platform plumbing for the folder feature; absent on drilled-in levels.
  final PlatformAccess? access;

  /// Persists a folder choice and swaps the root (null = app storage).
  final Future<void> Function(String?)? onSetDecksDir;

  /// The active theme id (per `themeById`); null resolves to the dark
  /// default. Drives the theme sheet's current-marker.
  final String? currentThemeId;

  /// Persists a theme choice and re-themes the whole app live.
  final Future<void> Function(String?)? onSetTheme;

  /// The support dir the pairing sheet reads and writes settings from;
  /// null uses the real app support dir. Tests inject a temp one.
  final Directory? supportDir;

  /// Builds the probe client the pairing sheet and the generate sheet use;
  /// null uses [HttpServerClient]. Tests inject a fake.
  final ServerClient Function(ServerConfig)? buildClient;

  /// How often the generate sheet polls the paired desktop while it is
  /// still working; null uses the sheet's own default. Tests shrink it.
  final Duration? generatePollInterval;

  /// Set only by [PickerScreen.mastered]: a fixed pre-filtered list of
  /// mastered decks, skipping the bridge listing call.
  final List<DeckEntry>? masteredEntries;

  @override
  State<PickerScreen> createState() => _PickerScreenState();
}

class _PickerScreenState extends State<PickerScreen> {
  late List<DeckEntry> _entries;

  /// Syncthing conflict copies next to any store under the root; a loud
  /// banner until dismissed for this visit.
  List<String> _conflicts = const [];
  bool _conflictsDismissed = false;

  /// Whether this instance is the Mastered window (see
  /// [PickerScreen.mastered]), not the ordinary root/drill-in listing.
  bool get _isMasteredView => widget.masteredEntries != null;

  /// The paired desktop, if any: gates the "Generate deck…" menu item.
  /// Loaded once on mount and refreshed after the pairing sheet closes (a
  /// pair/unpair while this screen is up must not leave a stale gate).
  ServerConfig? _pairedConfig;

  @override
  void initState() {
    super.initState();
    _load();
    _loadPairing();
  }

  Future<void> _loadPairing() async {
    final support = await _support();
    if (!mounted) return;
    setState(() => _pairedConfig = readServer(support));
  }

  void _load() {
    final fixed = widget.masteredEntries;
    if (fixed != null) {
      _entries = fixed;
      return;
    }
    final dir = widget.dir;
    _entries = dir == null
        ? listRoot(root: widget.root)
        : listMembers(root: widget.root, dir: dir);
    if (dir == null) {
      _conflicts = syncConflicts(root: widget.root);
    }
  }

  /// Opens a review session. `depth: null` (the tap path) lets the core
  /// resolve the deck's remembered depth, or its default when it has none:
  /// no sheet, no per-tap prompt.
  Future<void> _openDeck(DeckEntry entry, {Depth? depth}) async {
    if (!mounted) return;
    await Navigator.of(context).push(
      MaterialPageRoute(
        builder: (_) => ReviewScreen(
          deckPath: entry.path,
          rootDir: widget.root,
          depth: depth,
          device: widget.device,
        ),
      ),
    );
    // Progress changed while reviewing; refresh the due dots.
    setState(_load);
  }

  /// Opens the on-device trace walk. Mirrors `_openDeck`'s shape (push,
  /// await the pop, refresh the due dots): a walked trace can graduate its
  /// checkpoints or gain exam mastery, either of which changes this list.
  Future<void> _openWalk(DeckEntry entry) async {
    if (!mounted) return;
    await Navigator.of(context).push(
      MaterialPageRoute(
        builder: (_) => WalkScreen(
          deckPath: entry.path,
          rootDir: widget.root,
          device: widget.device,
          buildClient: widget.buildClient,
        ),
      ),
    );
    setState(_load);
  }

  /// The long-press re-pick: `_pickDepth` with the deck's remembered depth
  /// highlighted, opening with whatever is chosen.
  Future<void> _rePickDepth(DeckEntry entry) async {
    final depth = await _pickDepth(context, selected: entry.lastDepth);
    if (depth == null || !mounted) return;
    await _openDeck(entry, depth: depth);
  }

  @override
  Widget build(BuildContext context) {
    final isRoot = widget.dir == null;
    // Only the root loose-deck list tucks mastered decks away (item 13); a
    // workspace drill-in (item 14) keeps them in their dependency tree, and
    // the Mastered window itself is already the filtered list.
    final splitMastered = isRoot && !_isMasteredView;
    final active = splitMastered
        ? _entries.where((e) => !e.mastered).toList()
        : _entries;
    final mastered = splitMastered
        ? _entries.where((e) => e.mastered).toList()
        : const <DeckEntry>[];
    return Scaffold(
      appBar: AppBar(
        title: const AlixWordmark(),
        actions: [
          if (isRoot && widget.onSetDecksDir != null)
            PopupMenuButton<String>(
              onSelected: (choice) {
                if (choice == 'folder') _folderSheet();
                if (choice == 'pair') _pairSheet();
                if (choice == 'generate') _generateSheet();
                if (choice == 'theme') _themeSheet();
                if (choice == 'about') _about();
              },
              itemBuilder: (_) => [
                const PopupMenuItem(value: 'folder', child: Text('Decks folder…')),
                const PopupMenuItem(value: 'pair', child: Text('Pair with desktop…')),
                // Generate needs the paired desktop's AI; an unpaired phone
                // has nothing to ask, so the item is absent rather than a
                // dead button that would only fail (item T5.5).
                if (_pairedConfig != null)
                  const PopupMenuItem(value: 'generate', child: Text('Generate deck…')),
                const PopupMenuItem(value: 'theme', child: Text('Theme…')),
                const PopupMenuItem(value: 'about', child: Text('About')),
              ],
            ),
        ],
      ),
      body: Column(
        children: [
          if (widget.staleDecksDir != null)
            _notice(
              context,
              'Shared folder ${widget.staleDecksDir} is unavailable; '
              'using app storage for now.',
            ),
          if (_conflicts.isNotEmpty && !_conflictsDismissed)
            _conflictBanner(context),
          Expanded(
            child: ListView(
              padding: const EdgeInsets.fromLTRB(16, 16, 16, 24),
              children: [
                // The Mastered window's own eyebrow, else a drilled-into
                // workspace's name, matching the web picker's lede.
                if (_isMasteredView)
                  _lede(context, 'Mastered 🎉')
                else if (!isRoot && widget.title != null)
                  _lede(context, widget.title!),
                if (_entries.isEmpty)
                  _emptyHint(context)
                else ...[
                  for (final entry in active) _deckRow(context, entry),
                  if (mastered.isNotEmpty) _masteredAffordance(context, mastered),
                ],
              ],
            ),
          ),
        ],
      ),
    );
  }

  /// The web picker's cyan uppercase mono eyebrow.
  Widget _lede(BuildContext context, String text) {
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

  /// A deck or workspace as the web's bordered rounded row: a workspace's
  /// resolved emblem leads the title (else no leading icon), a chevron
  /// marks a drillable folder, and one trailing marker (trace / exam-due /
  /// due) reports the row's state. Tap opens at the remembered depth with
  /// no prompt; a deck row's long-press re-picks it (item 10). A workspace
  /// member (`entry.tree` non-empty) leads with its dependency-tree branch
  /// prefix instead of an icon; a locked member dims the whole row (browse
  /// stays allowed, only the tap's dimmed to signal the gate) (item 14).
  Widget _deckRow(BuildContext context, DeckEntry entry) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    final canRePick = !entry.isWorkspace && !entry.isTrace;
    return Padding(
      padding: const EdgeInsets.only(bottom: 6),
      child: Opacity(
        opacity: entry.locked ? 0.5 : 1,
        child: Material(
          color: Colors.transparent,
          child: InkWell(
            borderRadius: BorderRadius.circular(11),
            onTap: () => entry.isWorkspace
                ? _drillInto(entry)
                : entry.isTrace
                    ? _openWalk(entry)
                    : _openDeck(entry),
            onLongPress: canRePick ? () => _rePickDepth(entry) : null,
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
                    _emblem(entry.icon!, tokens),
                    const SizedBox(width: 10),
                  ],
                  if (entry.tree.isNotEmpty) ...[
                    Text(
                      entry.tree,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontFamily: 'monospace',
                        fontSize: 13,
                        color: tokens.faint,
                      ),
                    ),
                    const SizedBox(width: 6),
                  ],
                  Expanded(
                    child: Text(
                      entry.title,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: theme.textTheme.titleMedium
                          ?.copyWith(fontWeight: FontWeight.w600),
                    ),
                  ),
                  if (entry.isTrace) ...[
                    const SizedBox(width: 12),
                    Text(
                      'trace',
                      style: theme.textTheme.labelSmall?.copyWith(
                        color: tokens.faint,
                        fontFamily: 'monospace',
                        letterSpacing: 1.2,
                      ),
                    ),
                  ] else if (entry.examDue) ...[
                    // Drilled and awaiting its AI exam: the more actionable
                    // of the two states when a deck happens to also read
                    // due, so it wins the one trailing marker slot.
                    const SizedBox(width: 12),
                    Text(
                      'exam',
                      style: theme.textTheme.labelSmall?.copyWith(
                        color: tokens.warn,
                        fontFamily: 'monospace',
                        letterSpacing: 1.2,
                      ),
                    ),
                  ] else if (entry.due) ...[
                    const SizedBox(width: 12),
                    Icon(Icons.circle, size: 8, color: tokens.bolt),
                  ],
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

  /// The one row tucking mastered decks out of the ROOT list (item 13):
  /// styled distinctly (the good/celebration tint) with a count, opening the
  /// Mastered window. Only rendered when at least one mastered deck exists.
  Widget _masteredAffordance(BuildContext context, List<DeckEntry> mastered) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    return Padding(
      padding: const EdgeInsets.only(bottom: 6),
      child: Material(
        color: Colors.transparent,
        child: InkWell(
          borderRadius: BorderRadius.circular(11),
          onTap: () => _openMastered(mastered),
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
                    'Mastered · ${mastered.length}',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.titleMedium
                        ?.copyWith(fontWeight: FontWeight.w600, color: tokens.good),
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

  /// A workspace's picker emblem: a small leading glyph, tinted to the
  /// row's icon ink (mirrors the web picker's CSS-mask recolor). Constrained
  /// to the row's icon size so a hand-authored file cannot blow up the row.
  Widget _emblem(String path, AlixTokens tokens) {
    const size = 22.0;
    if (path.toLowerCase().endsWith('.svg')) {
      return SvgPicture.file(
        File(path),
        width: size,
        height: size,
        colorFilter: ColorFilter.mode(tokens.dim, BlendMode.srcIn),
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

  void _drillInto(DeckEntry entry) {
    Navigator.of(context).push(
      MaterialPageRoute(
        builder: (_) => PickerScreen(
          root: widget.root,
          dir: entry.path,
          title: entry.title,
          device: widget.device,
        ),
      ),
    );
  }

  void _openMastered(List<DeckEntry> mastered) {
    Navigator.of(context).push(
      MaterialPageRoute(
        builder: (_) => PickerScreen.mastered(
          root: widget.root,
          entries: mastered,
          device: widget.device,
        ),
      ),
    );
  }

  Widget _emptyHint(BuildContext context) {
    final theme = Theme.of(context);
    return Text(
      widget.dir == null
          ? 'No decks here yet. Put .txt decks in this folder, or choose a '
              'shared folder from the menu.'
          : 'no decks here',
      style: theme.textTheme.bodyMedium?.copyWith(color: theme.alix.dim),
    );
  }

  /// A quiet one-line notice (per-launch state, not dismissible).
  Widget _notice(BuildContext context, String text) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
      child: Text(
        text,
        style: theme.textTheme.bodySmall
            ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
      ),
    );
  }

  /// The one loud surface: a sync fork needs resolving before reviewing.
  Widget _conflictBanner(BuildContext context) {
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
              'A sync conflict file sits next to your progress '
              '(${_conflicts.length}). Review on one device at a time and '
              'resolve it first; see the manual.',
              style: theme.textTheme.bodySmall
                  ?.copyWith(color: theme.colorScheme.onErrorContainer),
            ),
          ),
          IconButton(
            icon: const Icon(Icons.close, size: 18),
            onPressed: () => setState(() => _conflictsDismissed = true),
          ),
        ],
      ),
    );
  }

  /// The decks-folder sheet: current folder, one primary action, a ghost
  /// revert. Hidden below Android 11 (no honest way to reach a shared
  /// folder there).
  Future<void> _folderSheet() async {
    final access = widget.access;
    if (access == null) return;
    final supported = await access.supportsSharedFolders();
    if (!mounted) return;
    await showModalBottomSheet<void>(
      context: context,
      builder: (sheet) {
        final theme = Theme.of(sheet);
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
                  widget.root,
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
                    onPressed: () {
                      Navigator.of(sheet).pop();
                      _chooseShared();
                    },
                    child: const Text('Choose shared folder…'),
                  )
                else
                  Text(
                    'Shared folders need Android 11 or newer.',
                    style: theme.textTheme.bodySmall
                        ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
                  ),
                if (widget.sharedDir != null) ...[
                  const SizedBox(height: 8),
                  TextButton(
                    onPressed: () {
                      Navigator.of(sheet).pop();
                      widget.onSetDecksDir?.call(null);
                    },
                    child: const Text('Use app storage'),
                  ),
                ],
              ],
            ),
          ),
        );
      },
    );
  }

  /// The theme picker sheet: `alixThemes` grouped Dark/Light, each a name
  /// plus a [surface, bolt, good] swatch built from that theme's OWN data
  /// (mirrors the web gallery's [bg, accent, green] preview dots). The
  /// active theme (falling back to the dark default) carries the one
  /// current-marker. Tapping a theme applies + persists it live and closes
  /// the sheet.
  Future<void> _themeSheet() async {
    final onSetTheme = widget.onSetTheme;
    if (onSetTheme == null) return;
    final current = widget.currentThemeId ?? alixThemes.first.id;
    await showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      builder: (sheet) => SafeArea(
        child: SizedBox(
          height: MediaQuery.of(sheet).size.height * 0.75,
          child: ListView(
            key: const ValueKey('theme-sheet-list'),
            padding: const EdgeInsets.symmetric(vertical: 8),
            children: [
              for (final mode in const [Brightness.dark, Brightness.light]) ...[
                _themeGroupLabel(sheet, mode == Brightness.dark ? 'Dark' : 'Light'),
                for (final entry in alixThemes.where((t) => t.mode == mode))
                  _themeTile(sheet, entry, current: current, onSetTheme: onSetTheme),
              ],
            ],
          ),
        ),
      ),
    );
  }

  /// The sheet's small uppercase mono eyebrow, mirroring `_lede`.
  Widget _themeGroupLabel(BuildContext context, String label) {
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

  Widget _themeTile(
    BuildContext context,
    AlixTheme theme, {
    required String current,
    required Future<void> Function(String?) onSetTheme,
  }) {
    final tokens = Theme.of(context).alix;
    final isCurrent = theme.id == current;
    return ListTile(
      key: ValueKey('theme-tile-${theme.id}'),
      leading: _themeSwatch(theme),
      title: Text(theme.name, maxLines: 1, overflow: TextOverflow.ellipsis),
      trailing: isCurrent ? Icon(Icons.check, size: 18, color: tokens.bolt) : null,
      onTap: () {
        onSetTheme(theme.id);
        Navigator.of(context).pop();
      },
    );
  }

  /// A 3-color chip built from the theme's own data: surface + bolt + good,
  /// the same [bg, accent, green] triple the web's swatch uses.
  Widget _themeSwatch(AlixTheme theme) {
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
          _themeDot(tokens.bolt),
          const SizedBox(width: 4),
          _themeDot(tokens.good),
        ],
      ),
    );
  }

  Widget _themeDot(Color color) => Container(
    width: 8,
    height: 8,
    decoration: BoxDecoration(color: color, shape: BoxShape.circle),
  );

  Future<Directory> _support() async => widget.supportDir ?? await getApplicationSupportDirectory();

  /// Opens the pairing sheet (see pairing_sheet.dart) and refreshes the
  /// paired-desktop gate on close: the only pairing surface in the app.
  Future<void> _pairSheet() async {
    final support = await _support();
    if (!mounted) return;
    final message = await showPairingSheet(
      context,
      support: support,
      buildClient: widget.buildClient ?? HttpServerClient.new,
    );
    if (!mounted) return;
    // A pair/unpair here changes whether "Generate deck…" belongs in the
    // menu; re-read it so the gate never goes stale for this open screen.
    setState(() => _pairedConfig = readServer(support));
    if (message != null) _snack(message);
  }

  /// The generate sheet: URL + optional guidance, generated on the paired
  /// desktop (the phone never runs the AI itself), then placed on-device.
  /// The desktop only ever hands back text (the iron rule); this method
  /// owns the one local, on-device decision the server can't make: where to
  /// save it. Every exit frees the server's generation slot with
  /// `generateClose` -- either the sheet's own `dispose` (cancelled or
  /// failed before `done`) or this method (the dest pick was cancelled, or
  /// the deck was placed).
  Future<void> _generateSheet() async {
    final support = await _support();
    if (!mounted) return;
    final config = readServer(support);
    if (config == null) return; // the menu item is pairing-gated; a race here is a quiet no-op
    final client = (widget.buildClient ?? HttpServerClient.new)(config);

    final dto = await showModalBottomSheet<RemoteGenerate>(
      context: context,
      isScrollControlled: true,
      builder: (_) => _GenerateSheet(
        client: client,
        pollInterval: widget.generatePollInterval ?? const Duration(milliseconds: 400),
      ),
    );

    final deck = dto?.deck;
    final filename = dto?.filename;
    if (deck == null || filename == null) return; // cancelled or failed; the sheet's dispose closed the slot

    if (!mounted) {
      await client.generateClose().catchError((_) {});
      client.close();
      return;
    }

    // The phone (never the desktop) chooses the destination, per the iron
    // rule; default to the current decks root, reusing the same in-app
    // browser the folder sheet drives.
    final dest = await Navigator.of(context).push<String>(
      MaterialPageRoute(builder: (_) => FolderBrowser(start: widget.root)),
    );
    if (dest == null) {
      await client.generateClose().catchError((_) {});
      client.close();
      _snack('alix did not save the generated deck.');
      return;
    }

    final written = applyGeneratedDeck(decksDir: dest, filename: filename, text: deck);
    await client.generateClose().catchError((_) {});
    client.close();
    if (!mounted) return;
    _snack('saved as $written');
    setState(_load);
  }

  Future<void> _chooseShared() async {
    final access = widget.access;
    if (access == null) return;
    if (!await access.ensureAllFilesAccess()) {
      _snack('Allow "All files access" for alix on the settings page that '
          'just opened, then try again.');
      return;
    }
    if (!mounted) return;
    // Android browses in-app (the system SAF picker's DocumentsUI crashes on
    // some devices); the desktop dev vehicle keeps its native dialog.
    final dir = Platform.isAndroid
        ? await Navigator.of(context).push<String>(
            MaterialPageRoute(
              builder: (_) => const FolderBrowser(start: '/storage/emulated/0'),
            ),
          )
        : await access.pickDirectory();
    if (dir == null) {
      _snack('alix stays on its current decks folder.');
      return;
    }
    await widget.onSetDecksDir?.call(dir);
  }

  void _snack(String text) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(text)));
  }

  /// App and embedded-core versions side by side; the app's from the
  /// installed package (never drifts from the build), the core's across
  /// the bridge.
  Future<void> _about() async {
    final app = await widget.access?.appVersion();
    if (!mounted) return;
    final tokens = Theme.of(context).alix;
    final dimStyle = Theme.of(context).textTheme.bodySmall?.copyWith(color: tokens.dim);
    showAboutDialog(
      context: context,
      applicationName: 'alix',
      applicationVersion: 'mobile ${app ?? 'dev'} / core ${coreVersion()}',
      applicationIcon: Image.asset(
        'assets/icon/alix-192.png',
        width: 48,
        height: 48,
      ),
      applicationLegalese: 'MIT or Apache-2.0, at your option.',
      children: [
        const SizedBox(height: 16),
        Text(
          'Free and open source. Telling someone who studies is the best support.',
          style: dimStyle,
        ),
        SelectableText('https://github.com/sponsors/Alex6323', style: dimStyle),
      ],
    );
  }
}

/// The generate sheet's body: a URL + optional guidance field, a Generate
/// button, and (once submitted) a calm working state -- mirroring
/// `_PairSheet`'s shape: every failure (a local URL check, a refused start,
/// an error phase, an expired pairing, or an unreachable poll) shows one
/// inline line and leaves the sheet open, so the user can fix the input and
/// try again rather than getting bounced. Pops with the settled
/// [RemoteGenerate] once `phase == "done"` (deck text and a suggested file
/// name both present); the caller places the file. Pops with `null` when the
/// user cancels without ever reaching `done` (back, or tapping outside).
class _GenerateSheet extends StatefulWidget {
  const _GenerateSheet({required this.client, this.pollInterval = const Duration(milliseconds: 400)});

  /// Owned by the caller ([_PickerScreenState._generateSheet]), which keeps
  /// it open past this sheet's own lifetime for the dest-pick step; see
  /// [_GenerateSheetState._handedOff].
  final ServerClient client;

  /// How often to poll `GET /api/remote/generate` while the desktop is
  /// still working. Tests shrink this well below the default.
  final Duration pollInterval;

  @override
  State<_GenerateSheet> createState() => _GenerateSheetState();
}

class _GenerateSheetState extends State<_GenerateSheet> {
  final _urlController = TextEditingController();
  final _guidanceController = TextEditingController();
  Timer? _pollTimer;

  bool _busy = false;
  String? _message;
  int? _elapsed;

  /// Set right before popping with a `done` DTO: the caller now owns
  /// [widget.client] for the dest pick, placement, and the final
  /// `generateClose`. Every other exit (cancel, a terminal failure the user
  /// dismisses by leaving) is this sheet's own job to close, so `dispose`
  /// only skips it here.
  bool _handedOff = false;

  @override
  void dispose() {
    _pollTimer?.cancel();
    if (!_handedOff) {
      // Fire and forget, mirroring the exam screen's dispose: the slot is
      // dropped either way, and a reply landing after dispose (including a
      // thrown PairingExpired) must not surface as an unhandled error.
      widget.client.generateClose().catchError((_) {});
      widget.client.close();
    }
    _urlController.dispose();
    _guidanceController.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    final url = _urlController.text.trim();
    final scheme = Uri.tryParse(url)?.scheme;
    if (scheme != 'http' && scheme != 'https') {
      setState(() => _message = 'alix can only generate from an http:// or https:// URL.');
      return;
    }
    setState(() {
      _busy = true;
      _message = null;
      _elapsed = null;
    });
    final guidance = _guidanceController.text.trim();
    bool ok;
    try {
      ok = await widget.client.generateStart(url, guidance: guidance.isEmpty ? null : guidance);
    } on PairingExpired {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _message = 'Pairing expired. Pair again from the deck list menu.';
      });
      return;
    }
    if (!mounted) return;
    if (!ok) {
      setState(() {
        _busy = false;
        _message = 'The desktop refused to generate this deck.';
      });
      return;
    }
    await _poll();
  }

  Future<void> _poll() async {
    RemoteGenerate? dto;
    try {
      dto = await widget.client.generateGet();
    } on PairingExpired {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _message = 'Pairing expired. Pair again from the deck list menu.';
      });
      return;
    }
    if (!mounted) return;
    if (dto == null) {
      setState(() {
        _busy = false;
        _message = 'Lost contact with the desktop.';
      });
      return;
    }
    if (dto.phase == 'error') {
      final error = dto.error ?? 'The desktop failed to generate this deck.';
      setState(() {
        _busy = false;
        _message = error;
      });
      return;
    }
    final settled = dto;
    if (settled.phase == 'done' && settled.deck != null && settled.filename != null) {
      _handedOff = true;
      Navigator.of(context).pop(settled);
      return;
    }
    // Still working (an open phase vocabulary; anything but done/error
    // counts as "keep polling", mirroring the exam screen).
    setState(() => _elapsed = settled.elapsed);
    _pollTimer = Timer(widget.pollInterval, () => _poll());
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return SafeArea(
      child: Padding(
        padding: EdgeInsets.fromLTRB(24, 24, 24, 24 + MediaQuery.of(context).viewInsets.bottom),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text('Generate deck', style: theme.textTheme.titleMedium),
            const SizedBox(height: 8),
            if (_busy)
              Text(
                _elapsed != null ? 'The desktop is working… ${_elapsed}s' : 'The desktop is working…',
                style: theme.textTheme.bodySmall?.copyWith(color: theme.colorScheme.onSurfaceVariant),
              )
            else ...[
              TextField(
                key: const ValueKey('generate-url-field'),
                controller: _urlController,
                decoration: const InputDecoration(labelText: 'URL', hintText: 'https://...'),
                maxLines: 1,
              ),
              const SizedBox(height: 12),
              TextField(
                key: const ValueKey('generate-guidance-field'),
                controller: _guidanceController,
                decoration: const InputDecoration(labelText: 'Guidance (optional)'),
                maxLines: 1,
              ),
              const SizedBox(height: 12),
              FilledButton(
                onPressed: _submit,
                child: const Text('Generate'),
              ),
              if (_message != null) ...[
                const SizedBox(height: 8),
                Text(
                  _message!,
                  style: theme.textTheme.bodySmall?.copyWith(color: theme.colorScheme.error),
                ),
              ],
            ],
          ],
        ),
      ),
    );
  }
}

/// The session depth pick (the long-press re-pick); [selected], when given,
/// gets one calm check-mark leading its tile as the current choice.
Future<Depth?> _pickDepth(BuildContext context, {Depth? selected}) {
  return showModalBottomSheet<Depth>(
    context: context,
    builder: (context) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          for (final (depth, label, hint) in [
            (Depth.recognize, 'Recognize', 'pick the answer out of four'),
            (Depth.recall, 'Recall', 'the everyday review'),
            (Depth.reconstruct, 'Reconstruct', 'type or rebuild the answer'),
          ])
            ListTile(
              leading: SizedBox(
                width: 22,
                child: depth == selected
                    ? Icon(Icons.check, size: 18, color: Theme.of(context).alix.bolt)
                    : null,
              ),
              title: Text(label),
              subtitle: Text(hint),
              onTap: () => Navigator.of(context).pop(depth),
            ),
        ],
      ),
    ),
  );
}
