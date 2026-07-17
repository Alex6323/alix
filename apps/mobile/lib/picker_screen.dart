import 'dart:io';

import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/folder_browser.dart';
import 'package:alix_mobile/platform_access.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/src/rust/api/listing.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/api/simple.dart';

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
    this.supportDir,
    this.buildClient,
  });

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

  /// The support dir the pairing sheet reads and writes settings from;
  /// null uses the real app support dir. Tests inject a temp one.
  final Directory? supportDir;

  /// Builds the probe client the pairing sheet uses; null uses
  /// [HttpServerClient]. Tests inject a fake.
  final ServerClient Function(ServerConfig)? buildClient;

  @override
  State<PickerScreen> createState() => _PickerScreenState();
}

class _PickerScreenState extends State<PickerScreen> {
  late List<DeckEntry> _entries;

  /// Syncthing conflict copies next to any store under the root; a loud
  /// banner until dismissed for this visit.
  List<String> _conflicts = const [];
  bool _conflictsDismissed = false;

  @override
  void initState() {
    super.initState();
    _load();
  }

  void _load() {
    final dir = widget.dir;
    _entries = dir == null
        ? listRoot(root: widget.root)
        : listMembers(root: widget.root, dir: dir);
    if (dir == null) {
      _conflicts = syncConflicts(root: widget.root);
    }
  }

  Future<void> _openDeck(DeckEntry entry) async {
    final depth = await _pickDepth(context);
    if (depth == null || !mounted) return;
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

  @override
  Widget build(BuildContext context) {
    final isRoot = widget.dir == null;
    return Scaffold(
      appBar: AppBar(
        title: const AlixWordmark(),
        actions: [
          if (isRoot && widget.onSetDecksDir != null)
            PopupMenuButton<String>(
              onSelected: (choice) {
                if (choice == 'folder') _folderSheet();
                if (choice == 'pair') _pairSheet();
                if (choice == 'about') _about();
              },
              itemBuilder: (_) => const [
                PopupMenuItem(value: 'folder', child: Text('Decks folder…')),
                PopupMenuItem(value: 'pair', child: Text('Pair with desktop…')),
                PopupMenuItem(value: 'about', child: Text('About')),
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
                // Drilled into a workspace: its name as the cyan eyebrow,
                // matching the web picker's lede.
                if (!isRoot && widget.title != null) _lede(context, widget.title!),
                if (_entries.isEmpty)
                  _emptyHint(context)
                else
                  for (final entry in _entries) _deckRow(context, entry),
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

  /// A deck or workspace as the web's bordered rounded row: no file icons,
  /// a chevron marks a drillable folder, a cyan dot marks something due.
  Widget _deckRow(BuildContext context, DeckEntry entry) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    return Padding(
      padding: const EdgeInsets.only(bottom: 6),
      child: Material(
        color: Colors.transparent,
        child: InkWell(
          borderRadius: BorderRadius.circular(11),
          onTap: () => entry.isWorkspace
              ? _drillInto(entry)
              : entry.isTrace
                  ? _traceNotice()
                  : _openDeck(entry),
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
    );
  }

  /// A trace deck is a predict-and-verify walk; the phone has no walk yet,
  /// and opening a review session on one is refused by the core. Say so
  /// calmly instead of white-screening.
  void _traceNotice() {
    ScaffoldMessenger.of(context).showSnackBar(const SnackBar(
      content: Text('Trace decks are guided source walks; for now they '
          'live in the web app.'),
    ));
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

  Future<Directory> _support() async => widget.supportDir ?? await getApplicationSupportDirectory();

  /// The pairing sheet: paste the URL `alix --lan` prints, probe it, and
  /// persist on success. The only pairing surface in the app; a failed
  /// probe shows one inline line, never a dialog.
  Future<void> _pairSheet() async {
    final support = await _support();
    if (!mounted) return;
    final message = await showModalBottomSheet<String>(
      context: context,
      isScrollControlled: true,
      builder: (sheet) => _PairSheet(
        support: support,
        current: readServer(support),
        buildClient: widget.buildClient ?? HttpServerClient.new,
      ),
    );
    if (message != null) _snack(message);
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
    );
  }
}

/// The pairing sheet's body: a paste field, a Pair button, an inline status
/// line, and, while paired, the current host:port with a ghost Unpair
/// button. Pops with a SnackBar message on success (paired or unpaired),
/// or stays open showing `_status` on any failure.
class _PairSheet extends StatefulWidget {
  const _PairSheet({
    required this.support,
    required this.current,
    required this.buildClient,
  });

  final Directory support;
  final ServerConfig? current;
  final ServerClient Function(ServerConfig) buildClient;

  @override
  State<_PairSheet> createState() => _PairSheetState();
}

class _PairSheetState extends State<_PairSheet> {
  final _controller = TextEditingController();
  String? _status;
  bool _busy = false;

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  Future<void> _pair() async {
    final parsed = parsePairingUrl(_controller.text);
    if (parsed == null) {
      setState(() => _status = 'that does not look like an alix pairing URL');
      return;
    }
    setState(() {
      _busy = true;
      _status = null;
    });
    final client = widget.buildClient(parsed);
    String? version;
    var refused = false;
    try {
      version = await client.version();
    } on PairingExpired {
      // alix answered and rejected the token: the pasted URL is stale (a
      // restarted server mints a fresh token). Say so distinctly; "no alix
      // answered" would send the user chasing the wrong problem.
      refused = true;
    } finally {
      client.close();
    }
    if (!mounted) return;
    if (refused) {
      setState(() {
        _busy = false;
        _status = 'alix answered but refused this token. '
            'Copy a fresh pairing URL from the server.';
      });
      return;
    }
    if (version == null) {
      setState(() {
        _busy = false;
        _status = 'no alix answered at ${parsed.host}:${parsed.port}';
      });
      return;
    }
    if (compareVersions(version, minServerVersion) < 0) {
      setState(() {
        _busy = false;
        _status = 'alix $version found, this app needs $minServerVersion or newer';
      });
      return;
    }
    await setServer(parsed, support: widget.support);
    if (!mounted) return;
    Navigator.of(context).pop('Paired with ${parsed.host}');
  }

  Future<void> _unpair() async {
    await setServer(null, support: widget.support);
    if (!mounted) return;
    Navigator.of(context).pop('Unpaired');
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final current = widget.current;
    return SafeArea(
      child: Padding(
        padding: EdgeInsets.fromLTRB(24, 24, 24, 24 + MediaQuery.of(context).viewInsets.bottom),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text('Pair with desktop', style: theme.textTheme.titleMedium),
            const SizedBox(height: 8),
            if (current != null) ...[
              Text(
                'Paired with ${current.host}:${current.port}',
                style: theme.textTheme.bodySmall?.copyWith(
                  color: theme.colorScheme.onSurfaceVariant,
                  fontFamily: 'monospace',
                ),
              ),
              const SizedBox(height: 16),
            ],
            TextField(
              key: const ValueKey('pairing-url-field'),
              controller: _controller,
              decoration: const InputDecoration(
                labelText: 'Pairing URL',
                hintText: 'http://<ip>:<port>/?token=...',
              ),
              maxLines: 1,
            ),
            const SizedBox(height: 12),
            FilledButton(
              onPressed: _busy ? null : _pair,
              child: Text(_busy ? 'Pairing…' : 'Pair'),
            ),
            if (_status != null) ...[
              const SizedBox(height: 8),
              Text(
                _status!,
                style: theme.textTheme.bodySmall?.copyWith(color: theme.colorScheme.error),
              ),
            ],
            if (current != null) ...[
              const SizedBox(height: 8),
              TextButton(
                onPressed: _busy ? null : _unpair,
                child: const Text('Unpair'),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

/// The session depth pick, defaulting to Recall.
Future<Depth?> _pickDepth(BuildContext context) {
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
              title: Text(label),
              subtitle: Text(hint),
              onTap: () => Navigator.of(context).pop(depth),
            ),
        ],
      ),
    ),
  );
}
