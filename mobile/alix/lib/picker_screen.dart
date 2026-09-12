import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/bridge/picker_bridge.dart';
import 'package:alix_mobile/bridge/sync_bridge.dart' as sync_bridge;
import 'package:alix_mobile/folder_browser.dart';
import 'package:alix_mobile/pairing_sheet.dart';
import 'package:alix_mobile/picker/generate_controller.dart';
import 'package:alix_mobile/picker/generate_sheet.dart';
import 'package:alix_mobile/picker/picker_controller.dart';
import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/picker_port.dart';
import 'package:alix_mobile/picker/picker_view.dart';
import 'package:alix_mobile/picker/picker_widgets.dart';
import 'package:alix_mobile/platform_access.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/settings_screen.dart';
import 'package:alix_mobile/sync/root_switcher_sheet.dart';
import 'package:alix_mobile/sync/sync_controller.dart';
import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync/sync_port.dart';
import 'package:alix_mobile/sync/sync_sheet.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/walk_screen.dart';

class PickerScreen extends StatefulWidget {
  const PickerScreen({
    super.key,
    required this.root,
    this.dir,
    this.title,
    this.device,
    this.access,
    this.currentThemeId,
    this.onSetTheme,
    this.supportDir,
    this.buildClient,
    this.generatePollInterval,
    this.syncController,
    this.buildSyncPort,
    this.isPairedSubtree = false,
    this.isPushedRoute = false,
  }) : masteredEntries = null;

  const PickerScreen.mastered({
    super.key,
    required this.root,
    required List<PickerEntry> entries,
    this.device,
    this.syncController,
  }) : masteredEntries = entries,
       dir = null,
       title = null,
       access = null,
       currentThemeId = null,
       onSetTheme = null,
       supportDir = null,
       buildClient = null,
       generatePollInterval = null,
       buildSyncPort = null,
       isPairedSubtree = false,
       isPushedRoute = true;

  final String root;
  final String? dir;
  final String? title;
  final String? device;
  final PlatformAccess? access;
  final String? currentThemeId;
  final Future<void> Function(String?)? onSetTheme;
  final Directory? supportDir;
  final ServerClient Function(ServerConfig)? buildClient;
  final Duration? generatePollInterval;
  final List<PickerEntry>? masteredEntries;

  /// The active paired root's sync controller, forwarded from the
  /// screen that first built it (the root mount or a root switch) down
  /// through a drill-in or the mastered view, so every depth shares one
  /// running cycle rather than each starting its own. Null while unpaired
  /// or showing a root that is not the paired one.
  final SyncController? syncController;

  /// Builds the port a freshly-mounted screen's sync controller talks to.
  /// Tests inject a fake; the real bridge otherwise.
  final SyncPort Function(ServerConfig config, String rootDir)? buildSyncPort;

  /// Whether [root] (and [dir], when set) is the active paired desktop's
  /// tree rather than the phone's own: a drill-in forwards this from
  /// whichever section its parent row came from. False for the root
  /// screen's own phone-own list and for the mastered view.
  final bool isPairedSubtree;

  /// Whether this instance was reached via [Navigator.push] (a drill-in or
  /// the mastered view), so its app bar owes a real back button. False only
  /// for the one instance `main.dart` mounts as `home:`. `Navigator.of
  /// (context).canPop()` is not a substitute: it answers for the whole
  /// stack, not for this route, and reads true on the root screen too
  /// while an unrelated route above it is still popping.
  final bool isPushedRoute;

  @override
  State<PickerScreen> createState() => _PickerScreenState();
}

class _PickerScreenState extends State<PickerScreen> {
  late final PickerPort _port;
  late final PickerController _controller;
  SyncController? _syncController;
  bool _hasPairings = false;

  /// The active pairing's directory on disk, set once `_loadPairing`
  /// resolves it at the root screen; null while unpaired.
  String? _pairedDir;

  /// The active pairing's "host:port" label for the paired section's group
  /// heading; null alongside [_pairedDir].
  String? _pairedLabel;

  /// Every port-binding field of the pairing [_syncController] was built
  /// for (root-screen-owned instances only): scheme, host, port, token,
  /// and root id. Two served roots can deliberately share one configured
  /// token, so the token alone is not enough identity; a re-pair that
  /// changes any of these fields must rebuild the port and controller, not
  /// keep dialing the stale endpoint or root.
  ({String scheme, String host, int port, String token, String rootId})?
  _syncControllerKey;

  @override
  void initState() {
    super.initState();
    _port = const PickerBridge();
    _controller = PickerController(
      port: _port,
      root: widget.root,
      dir: widget.dir,
      masteredEntries: widget.masteredEntries,
    );
    final forwarded = widget.syncController;
    if (forwarded != null) _attachSyncController(forwarded);
    _loadPairing();
  }

  @override
  void dispose() {
    _syncController?.removeListener(_controller.reload);
    _controller.dispose();
    // Only dispose a controller this screen built itself; one forwarded
    // through widget.syncController is still owned by whichever screen
    // built it, and keeps running for that screen and any of its own
    // descendants.
    if (widget.syncController == null) _syncController?.dispose();
    super.dispose();
  }

  /// `_controller` is the one listenable `build()`'s `ListenableBuilder`
  /// ever subscribes to, so a sync controller discovered after the first
  /// frame (the paired root is only known once `_loadPairing` resolves)
  /// still needs a way to trigger a rebuild: every notification from
  /// [controller] is forwarded through `_controller.reload()`, which
  /// `ListenableBuilder` was already listening to from the start.
  void _attachSyncController(SyncController controller) {
    _syncController = controller;
    controller.addListener(_controller.reload);
  }

  /// Detaches [_syncController] from `_controller.reload`, disposes it
  /// (closing its port), and clears it and [_syncControllerKey]. Shared by
  /// a re-pair (before attaching the freshly built replacement) and an
  /// unpair (no replacement follows); both run only for the root screen
  /// that owns this controller.
  void _detachSyncController() {
    final stale = _syncController;
    if (stale != null) {
      stale.removeListener(_controller.reload);
      stale.dispose();
    }
    _syncController = null;
    _syncControllerKey = null;
  }

  Future<void> _loadPairing() async {
    final support = await _support();
    _hasPairings = readPairings(support).isNotEmpty;
    final isPairedRootScreen =
        widget.dir == null && widget.masteredEntries == null;
    final config = readActivePairing(support);
    if (config == null) {
      if (isPairedRootScreen) {
        _pairedDir = null;
        _pairedLabel = null;
        _controller.setPairedRoot(null);
        _detachSyncController();
        if (mounted) _controller.reload();
      }
      if (mounted) _controller.setServerReachable(false);
      return;
    }
    final String pairedDir;
    try {
      pairedDir = sync_bridge.pairedRootDirFor(
        support: support.path,
        rootId: config.rootId,
      );
    } on Object {
      if (isPairedRootScreen) {
        _pairedDir = null;
        _pairedLabel = null;
        _controller.setPairedRoot(null);
        _detachSyncController();
        if (mounted) _controller.reload();
      }
      if (mounted) _controller.setServerReachable(false);
      return;
    }

    String? recoveryError;
    if (isPairedRootScreen) {
      try {
        sync_bridge.pairedRecoverFor(rootDir: pairedDir);
      } on Object catch (error) {
        recoveryError = 'could not prepare the paired folder: $error';
      }
      _pairedDir = pairedDir;
      _pairedLabel = '${config.host}:${config.port}';
      _controller.setPairedRoot(pairedDir);
    }

    final syncControllerKey = (
      scheme: config.scheme,
      host: config.host,
      port: config.port,
      token: config.token,
      rootId: config.rootId,
    );
    final freshlyBuilt =
        isPairedRootScreen &&
        (_syncController == null || _syncControllerKey != syncControllerKey);
    if (freshlyBuilt) {
      _detachSyncController();
      final buildPort =
          widget.buildSyncPort ??
          (config, rootDir) =>
              sync_bridge.SyncBridgePort(config: config, rootDir: rootDir);
      _syncControllerKey = syncControllerKey;
      _attachSyncController(
        SyncController(
          port: buildPort(config, pairedDir),
          initialError: recoveryError,
        ),
      );
      if (mounted) _controller.reload();
    }

    final client = (widget.buildClient ?? HttpServerClient.new)(config);
    ServerVersion? probe;
    try {
      probe = await client.version();
    } on PairingExpired {
      probe = null;
    } finally {
      client.close();
    }
    final live =
        probe != null && compareVersions(probe.version, minServerVersion) >= 0;
    if (mounted) _controller.setServerReachable(live);

    if (freshlyBuilt && live && probe.rootId == config.rootId) {
      final syncController = _syncController!;
      unawaited(
        syncController.cycle().then((_) {
          if (mounted) _controller.reload();
        }),
      );
    }
  }

  void _syncEntry(PickerEntry entry) {
    _syncController?.cycle(entry: _entryFileName(entry.path));
  }

  /// The manifest name a picker row's path implies: the file or directory
  /// name relative to its root, exactly what `SyncEntry.name` carries. Not
  /// [PickerEntry.title], which is the display name (falls back to a bare
  /// file stem for a loose deck, or `alix.toml`'s `title` for a workspace).
  String _entryFileName(String path) {
    final normalized = path.replaceAll('\\', '/');
    final trimmed = normalized.endsWith('/')
        ? normalized.substring(0, normalized.length - 1)
        : normalized;
    return trimmed.split('/').last;
  }

  void _pullAvailable(String name) {
    _syncController?.cycle(entry: name);
  }

  Future<void> _openSyncReport() async {
    final syncController = _syncController;
    if (syncController == null) return;
    final report = syncController.lastReport;
    final conflicts = syncController.pendingConflicts;
    final unpushedOrphans = syncController.unpushedEntries;
    await showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      builder: (_) => SyncReportSheet(
        report: report,
        conflicts: conflicts,
        unpushedOrphans: unpushedOrphans,
        onResolve: (deckId, keepPhone) =>
            syncController.resolve(deckId, keepPhone: keepPhone),
        onRemoveOrphan: syncController.removeOrphan,
      ),
    );
    syncController.markReportRead();
  }

  Future<void> _openConflictChoice(SyncPendingConflict conflict) async {
    final syncController = _syncController;
    if (syncController == null || !mounted) return;
    await showConflictChoiceSheet(
      context,
      conflict: conflict,
      onResolve: (deckId, keepPhone) =>
          syncController.resolve(deckId, keepPhone: keepPhone),
    );
  }

  Future<void> _rootSwitcherSheet() async {
    final support = await _support();
    if (!mounted) return;
    final pairings = readPairings(support);
    final active = readActivePairing(support);
    final chosen = await showRootSwitcherSheet(
      context,
      pairings: pairings,
      activeRootId: active?.rootId,
    );
    if (chosen == null) return;
    await setActiveRoot(chosen.rootId, support: support);
    if (!mounted) return;
    // The phone's own root never changes on a switch: only which paired
    // desktop's entries the second section shows, resolved fresh by the
    // new screen's own _loadPairing.
    Navigator.of(context).pushReplacement(
      MaterialPageRoute(
        builder: (_) => PickerScreen(
          root: widget.root,
          device: widget.device,
          access: widget.access,
          currentThemeId: widget.currentThemeId,
          onSetTheme: widget.onSetTheme,
          supportDir: widget.supportDir,
          buildClient: widget.buildClient,
          generatePollInterval: widget.generatePollInterval,
          buildSyncPort: widget.buildSyncPort,
        ),
      ),
    );
  }

  void _openSettings() {
    Navigator.of(context).push(
      PageRouteBuilder<void>(
        transitionDuration: const Duration(milliseconds: 280),
        reverseTransitionDuration: const Duration(milliseconds: 240),
        pageBuilder: (_, _, _) => SettingsScreen(
          onSupport: _supportSheet,
          onConnectedDevices: _pairSheet,
          onTheme: _themeSheet,
          onAbout: _about,
          onGenerate: _controller.serverReachable ? _generateSheet : null,
          onPairedDesktop: _hasPairings ? _rootSwitcherSheet : null,
        ),
        transitionsBuilder: (_, animation, _, child) {
          final curved = CurvedAnimation(
            parent: animation,
            curve: Curves.easeOutCubic,
            reverseCurve: Curves.easeInCubic,
          );
          return SlideTransition(
            position: Tween<Offset>(
              begin: const Offset(0, 1),
              end: Offset.zero,
            ).animate(curved),
            child: child,
          );
        },
      ),
    );
  }

  Future<void> _supportSheet() async {
    await showModalBottomSheet<void>(
      context: context,
      builder: (_) => const PickerSupportSheet(),
    );
  }

  Future<void> _openDeck(
    PickerEntry entry, {
    required String root,
    required bool isPaired,
    PickerDepth? depth,
  }) async {
    if (!mounted) return;
    final syncController = _syncController;
    if (isPaired && syncController != null) {
      final deckId = deckIdForPath(
        entries: syncController.pairedEntries,
        rootDir: root,
        path: entry.path,
      );
      final matches = deckId == null
          ? const <SyncPendingConflict>[]
          : syncController.pendingConflicts.where((c) => c.deckId == deckId);
      if (matches.isNotEmpty) {
        await _openConflictChoice(matches.first);
        return;
      }
    }
    if (depth == null &&
        entry.lastDepth == PickerDepth.recognize &&
        !entry.canRecognize) {
      depth = PickerDepth.recall;
    }
    await Navigator.of(context).push(
      MaterialPageRoute(
        builder: (_) => ReviewScreen(
          deckPath: entry.path,
          rootDir: root,
          depth: switch (depth) {
            PickerDepth.recognize => ReviewDepth.recognize,
            PickerDepth.recall => ReviewDepth.recall,
            PickerDepth.reconstruct => ReviewDepth.reconstruct,
            null => null,
          },
          device: widget.device,
          supportDir: widget.supportDir,
          buildClient: widget.buildClient,
          syncController: isPaired ? syncController : null,
        ),
      ),
    );
    _controller.reload();
  }

  Future<void> _openWalk(PickerEntry entry, {required String root}) async {
    if (!mounted) return;
    await Navigator.of(context).push(
      MaterialPageRoute(
        builder: (_) => WalkScreen(
          deckPath: entry.path,
          rootDir: root,
          device: widget.device,
          buildClient: widget.buildClient,
        ),
      ),
    );
    _controller.reload();
  }

  Future<void> _rePickDepth(
    PickerEntry entry, {
    required String root,
    required bool isPaired,
  }) async {
    final depth = await showModalBottomSheet<PickerDepth>(
      context: context,
      builder: (sheet) => PickerDepthSheet(
        selected: entry.lastDepth,
        canRecognize: entry.canRecognize,
        onChoose: (depth) => Navigator.of(sheet).pop(depth),
      ),
    );
    if (depth == null || !mounted) return;
    await _openDeck(entry, root: root, isPaired: isPaired, depth: depth);
  }

  String _ymd(DateTime value) =>
      '${value.year.toString().padLeft(4, '0')}'
      '-${value.month.toString().padLeft(2, '0')}'
      '-${value.day.toString().padLeft(2, '0')}';

  Future<void> _deadlineSheet(PickerEntry entry) async {
    final current = entry.deadline;
    final action = await showModalBottomSheet<String>(
      context: context,
      builder: (sheet) => PickerDeadlineSheet(
        current: current,
        onPick: () => Navigator.of(sheet).pop('pick'),
        onClear: () => Navigator.of(sheet).pop('clear'),
      ),
    );
    if (!mounted) return;
    if (action == 'clear') {
      _controller.clearDeadline(entry.path);
      return;
    }
    if (action != 'pick') return;
    final today = DateTime.now();
    final currentDate = current == null
        ? null
        : DateTime.tryParse(current.date);
    final initial = (currentDate == null || currentDate.isBefore(today))
        ? today
        : currentDate;
    final picked = await showDatePicker(
      context: context,
      initialDate: initial,
      firstDate: today,
      lastDate: today.add(const Duration(days: 5 * 365)),
    );
    if (picked == null || !mounted) return;
    _controller.setDeadline(dir: entry.path, date: _ymd(picked));
  }

  void _openEntry(PickerEntry entry) =>
      _openEntryIn(entry, root: widget.root, isPaired: widget.isPairedSubtree);

  void _openPairedEntry(PickerEntry entry) {
    final pairedDir = _pairedDir;
    if (pairedDir == null) return;
    _openEntryIn(entry, root: pairedDir, isPaired: true);
  }

  void _openEntryIn(
    PickerEntry entry, {
    required String root,
    required bool isPaired,
  }) {
    // A progress-error row is a diagnostic, not an action: the core refuses
    // the open, so navigating would only strand the user in a dead screen.
    if (entry.progressError && !entry.isWorkspace) return;
    if (entry.isWorkspace) {
      _drillInto(entry, root: root, isPaired: isPaired);
    } else if (entry.isTrace) {
      _openWalk(entry, root: root);
    } else {
      _openDeck(entry, root: root, isPaired: isPaired);
    }
  }

  void _longPressEntry(PickerEntry entry) => _longPressEntryIn(
    entry,
    root: widget.root,
    isPaired: widget.isPairedSubtree,
  );

  void _longPressPairedEntry(PickerEntry entry) {
    final pairedDir = _pairedDir;
    if (pairedDir == null) return;
    _longPressEntryIn(entry, root: pairedDir, isPaired: true);
  }

  void _longPressEntryIn(
    PickerEntry entry, {
    required String root,
    required bool isPaired,
  }) {
    if (entry.progressError && !entry.isWorkspace) return;
    if (entry.isWorkspace) {
      _deadlineSheet(entry);
    } else if (!entry.isTrace) {
      _rePickDepth(entry, root: root, isPaired: isPaired);
    }
  }

  void _drillInto(
    PickerEntry entry, {
    required String root,
    required bool isPaired,
  }) {
    Navigator.of(context).push(
      MaterialPageRoute(
        builder: (_) => PickerScreen(
          root: root,
          dir: entry.path,
          title: entry.title,
          device: widget.device,
          syncController: _syncController,
          isPairedSubtree: isPaired,
          isPushedRoute: true,
        ),
      ),
    );
  }

  void _openMastered(List<PickerEntry> mastered) {
    Navigator.of(context).push(
      MaterialPageRoute(
        builder: (_) => PickerScreen.mastered(
          root: widget.root,
          entries: mastered,
          device: widget.device,
          syncController: _syncController,
        ),
      ),
    );
  }

  Future<void> _addTutorial() async {
    try {
      await _controller.addTutorial();
    } catch (_) {
      if (mounted) _snack('could not add the tutorial deck here.');
    }
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: _controller,
      builder: (context, _) {
        // Read fresh on every rebuild (not snapshotted above): this
        // callback re-runs on a plain _controller.reload() forwarded
        // from _syncController, so a stale local would still show the
        // pre-cycle state.
        final syncController = _syncController;
        final isPairedRootScreen =
            widget.dir == null && widget.masteredEntries == null;
        return PickerView(
          entries: _controller.entries,
          isLoading: _controller.isLoading,
          deadline: _controller.deadline,
          isRoot: widget.dir == null,
          isMasteredView: _controller.isMasteredView,
          leading: widget.isPushedRoute
              ? const BackButton()
              : widget.dir == null && widget.onSetTheme != null
              ? IconButton(
                  icon: const Icon(Icons.menu),
                  tooltip: 'Settings',
                  onPressed: _openSettings,
                )
              : const SizedBox(width: 56),
          title: widget.title,
          onOpenEntry: _openEntry,
          onLongPressEntry: _longPressEntry,
          onOpenMastered: _openMastered,
          onAddTutorial: _addTutorial,
          syncStatus: syncController?.statusLine,
          onOpenSyncReport: syncController == null ? null : _openSyncReport,
          pairedLabel: isPairedRootScreen ? _pairedLabel : null,
          pairedEntries: isPairedRootScreen
              ? _controller.pairedRootEntries
              : const [],
          onOpenPairedEntry: isPairedRootScreen ? _openPairedEntry : null,
          onLongPressPairedEntry: isPairedRootScreen
              ? _longPressPairedEntry
              : null,
          onSyncEntry: syncController != null && isPairedRootScreen
              ? _syncEntry
              : null,
          availableEntries: syncController != null && isPairedRootScreen
              ? syncController.availableEntries
              : const [],
          onPullAvailable: syncController != null && isPairedRootScreen
              ? _pullAvailable
              : null,
        );
      },
    );
  }

  Future<void> _themeSheet() async {
    final onSetTheme = widget.onSetTheme;
    if (onSetTheme == null) return;
    final current = widget.currentThemeId ?? alixThemes.first.id;
    await showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      builder: (sheet) => PickerThemeSheet(
        current: current,
        onChoose: (theme) {
          onSetTheme(theme);
          Navigator.of(sheet).pop();
        },
      ),
    );
  }

  Future<Directory> _support() async =>
      widget.supportDir ?? await getApplicationSupportDirectory();

  Future<void> _pairSheet() async {
    final support = await _support();
    if (!mounted) return;
    final message = await showPairingSheet(
      context,
      support: support,
      buildClient: widget.buildClient ?? HttpServerClient.new,
    );
    if (!mounted) return;
    if (message != null) _snack(message);
    unawaited(_loadPairing());
  }

  Future<void> _generateSheet() async {
    final support = await _support();
    if (!mounted) return;
    final config = readActivePairing(support);
    if (config == null) return;
    final client = (widget.buildClient ?? HttpServerClient.new)(config);
    final controller = GenerateController(
      client: client,
      pollInterval:
          widget.generatePollInterval ?? const Duration(milliseconds: 400),
    );

    final dto = await showModalBottomSheet<RemoteGenerate>(
      context: context,
      isScrollControlled: true,
      builder: (_) => GenerateSheet(controller: controller),
    );

    final deck = dto?.deck;
    final filename = dto?.filename;
    if (deck == null || filename == null) return;

    if (!mounted) {
      await client.generateClose().catchError((_) {});
      client.close();
      return;
    }

    final dest = await Navigator.of(context).push<String>(
      MaterialPageRoute(builder: (_) => FolderBrowser(start: widget.root)),
    );
    if (dest == null) {
      await client.generateClose().catchError((_) {});
      client.close();
      _snack('alix did not save the generated deck.');
      return;
    }

    final written = _port.applyGeneratedDeck(
      decksDir: dest,
      filename: filename,
      text: deck,
    );
    await client.generateClose().catchError((_) {});
    client.close();
    if (!mounted) return;
    _snack('saved as $written');
    _controller.reload();
  }

  void _snack(String text) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(text)));
  }

  Future<void> _about() async {
    final app = await widget.access?.appVersion();
    if (!mounted) return;
    showAboutDialog(
      context: context,
      applicationName: 'alix',
      applicationVersion: 'mobile ${app ?? 'dev'} / core ${_port.coreVersion}',
      applicationIcon: Image.asset(
        'assets/icon/alix-192.png',
        width: 48,
        height: 48,
      ),
      applicationLegalese: 'MIT or Apache-2.0, at your option.',
    );
  }
}
