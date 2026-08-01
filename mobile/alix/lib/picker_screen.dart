import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/bridge/picker_bridge.dart';
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
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/walk_screen.dart';

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

  const PickerScreen.mastered({
    super.key,
    required this.root,
    required List<PickerEntry> entries,
    this.device,
  }) : masteredEntries = entries,
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
  final String? device;
  final String? sharedDir;
  final String? staleDecksDir;
  final PlatformAccess? access;
  final Future<void> Function(String?)? onSetDecksDir;
  final String? currentThemeId;
  final Future<void> Function(String?)? onSetTheme;
  final Directory? supportDir;
  final ServerClient Function(ServerConfig)? buildClient;
  final Duration? generatePollInterval;
  final List<PickerEntry>? masteredEntries;

  @override
  State<PickerScreen> createState() => _PickerScreenState();
}

class _PickerScreenState extends State<PickerScreen> {
  late final PickerPort _port;
  late final PickerController _controller;

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
    _loadPairing();
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  Future<void> _loadPairing() async {
    final support = await _support();
    final config = readServer(support);
    if (config == null) {
      if (mounted) _controller.setServerReachable(false);
      return;
    }
    final client = (widget.buildClient ?? HttpServerClient.new)(config);
    String? version;
    try {
      version = await client.version();
    } on PairingExpired {
      version = null;
    } finally {
      client.close();
    }
    final live =
        version != null && compareVersions(version, minServerVersion) >= 0;
    if (mounted) _controller.setServerReachable(live);
  }

  void _openSettings() {
    Navigator.of(context).push(
      PageRouteBuilder<void>(
        transitionDuration: const Duration(milliseconds: 280),
        reverseTransitionDuration: const Duration(milliseconds: 240),
        pageBuilder: (_, _, _) => SettingsScreen(
          onSupport: _supportSheet,
          onConnectedDevices: _pairSheet,
          onDecksFolder: _folderSheet,
          onTheme: _themeSheet,
          onAbout: _about,
          onGenerate: _controller.serverReachable ? _generateSheet : null,
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

  Future<void> _openDeck(PickerEntry entry, {PickerDepth? depth}) async {
    if (!mounted) return;
    if (depth == null &&
        entry.lastDepth == PickerDepth.recognize &&
        !entry.canRecognize) {
      depth = PickerDepth.recall;
    }
    await Navigator.of(context).push(
      MaterialPageRoute(
        builder: (_) => ReviewScreen(
          deckPath: entry.path,
          rootDir: widget.root,
          depth: switch (depth) {
            PickerDepth.recognize => ReviewDepth.recognize,
            PickerDepth.recall => ReviewDepth.recall,
            PickerDepth.reconstruct => ReviewDepth.reconstruct,
            null => null,
          },
          device: widget.device,
        ),
      ),
    );
    _controller.reload();
  }

  Future<void> _openWalk(PickerEntry entry) async {
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
    _controller.reload();
  }

  Future<void> _rePickDepth(PickerEntry entry) async {
    final depth = await showModalBottomSheet<PickerDepth>(
      context: context,
      builder: (sheet) => PickerDepthSheet(
        selected: entry.lastDepth,
        canRecognize: entry.canRecognize,
        onChoose: (depth) => Navigator.of(sheet).pop(depth),
      ),
    );
    if (depth == null || !mounted) return;
    await _openDeck(entry, depth: depth);
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

  void _openEntry(PickerEntry entry) {
    if (entry.isWorkspace) {
      _drillInto(entry);
    } else if (entry.isTrace) {
      _openWalk(entry);
    } else {
      _openDeck(entry);
    }
  }

  void _longPressEntry(PickerEntry entry) {
    if (entry.isWorkspace) {
      _deadlineSheet(entry);
    } else if (!entry.isTrace) {
      _rePickDepth(entry);
    }
  }

  void _drillInto(PickerEntry entry) {
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

  void _openMastered(List<PickerEntry> mastered) {
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
      builder: (context, _) => PickerView(
        entries: _controller.entries,
        conflicts: _controller.conflicts,
        conflictsDismissed: _controller.conflictsDismissed,
        deadline: _controller.deadline,
        isRoot: widget.dir == null,
        isMasteredView: _controller.isMasteredView,
        leading: Navigator.of(context).canPop()
            ? const BackButton()
            : widget.dir == null && widget.onSetDecksDir != null
            ? IconButton(
                icon: const Icon(Icons.menu),
                tooltip: 'Settings',
                onPressed: _openSettings,
              )
            : const SizedBox(width: 56),
        title: widget.title,
        staleDecksDir: widget.staleDecksDir,
        onOpenEntry: _openEntry,
        onLongPressEntry: _longPressEntry,
        onOpenMastered: _openMastered,
        onAddTutorial: _addTutorial,
        onDismissConflicts: _controller.dismissConflicts,
      ),
    );
  }

  Future<void> _folderSheet() async {
    final access = widget.access;
    if (access == null) return;
    final supported = await access.supportsSharedFolders();
    if (!mounted) return;
    await showModalBottomSheet<void>(
      context: context,
      builder: (sheet) => PickerFolderSheet(
        root: widget.root,
        supported: supported,
        hasSharedDir: widget.sharedDir != null,
        onChoose: () {
          Navigator.of(sheet).pop();
          _chooseShared();
        },
        onUseAppStorage: () {
          Navigator.of(sheet).pop();
          widget.onSetDecksDir?.call(null);
        },
      ),
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
    final config = readServer(support);
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

  Future<void> _chooseShared() async {
    final access = widget.access;
    if (access == null) return;
    if (!await access.ensureAllFilesAccess()) {
      _snack(
        'Allow "All files access" for alix on the settings page that '
        'just opened, then try again.',
      );
      return;
    }
    if (!mounted) return;
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
