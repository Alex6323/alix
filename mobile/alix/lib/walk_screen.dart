import 'dart:io';

import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/bridge/walk_bridge.dart';
import 'package:alix_mobile/exam_screen.dart';
import 'package:alix_mobile/leave_guard.dart';
import 'package:alix_mobile/pairing_sheet.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/walk/walk_controller.dart';
import 'package:alix_mobile/walk/walk_models.dart';
import 'package:alix_mobile/walk/walk_view.dart';

/// The on-device trace walk route.
///
/// The screen owns Flutter lifecycle, navigation, pairing, and exam handoff.
/// Bridge-backed session state and named transitions live in [WalkController],
/// while the rendered tree lives below [WalkView].
class WalkScreen extends StatefulWidget {
  const WalkScreen({
    super.key,
    required this.deckPath,
    required this.rootDir,
    this.device,
    this.supportDir,
    this.buildClient,
  });

  final String deckPath;
  final String rootDir;

  /// This install's label for the store's last-writer marker.
  final String? device;

  /// The support dir the pairing config is read from. Tests inject a temp one.
  final Directory? supportDir;

  /// Builds the exam handoff's client. Tests inject a fake.
  final ServerClient Function(ServerConfig)? buildClient;

  @override
  State<WalkScreen> createState() => _WalkScreenState();
}

class _WalkScreenState extends State<WalkScreen> {
  late final WalkController _controller;
  final TextEditingController _predict = TextEditingController();

  ServerClient? _client;
  Directory? _support;

  @override
  void initState() {
    super.initState();
    _controller = WalkController(
      factory: const WalkBridgeFactory(),
      deckPath: widget.deckPath,
      rootDir: widget.rootDir,
      device: widget.device,
    );
    if (_controller.openError != null) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _bailToCaller());
    } else {
      _probeServer();
    }
  }

  @override
  void dispose() {
    _predict.dispose();
    _controller.dispose();
    _client?.close();
    super.dispose();
  }

  Future<void> _probeServer() async {
    final support = widget.supportDir ?? await getApplicationSupportDirectory();
    _support = support;
    final config = readServer(support);
    if (config == null) return;
    final client = (widget.buildClient ?? HttpServerClient.new)(config);
    String? version;
    try {
      version = await client.version();
    } on PairingExpired {
      client.close();
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: const Text(
            'Pairing expired. Pair again from Settings → Connected devices.',
          ),
          action: SnackBarAction(
            label: 'Re-pair',
            onPressed: () {
              if (!mounted) return;
              showPairingSheet(
                context,
                support: support,
                buildClient: widget.buildClient ?? HttpServerClient.new,
              );
            },
          ),
        ),
      );
      return;
    }
    final live =
        version != null && compareVersions(version, minServerVersion) >= 0;
    if (!live || !mounted) {
      client.close();
      return;
    }
    _client = client;
    _controller.setServerLive(true);
  }

  void _bailToCaller() {
    if (!mounted) return;
    final messenger = ScaffoldMessenger.of(context);
    final message = _controller.openError ?? 'this deck cannot be walked';
    Navigator.of(context).maybePop();
    messenger.showSnackBar(SnackBar(content: Text(message)));
  }

  void _restart() {
    _controller.restart();
    if (_controller.openError != null) {
      _bailToCaller();
    } else {
      _predict.clear();
    }
  }

  void _submitPredict() {
    _controller.predict(_predict.text);
    _predict.clear();
  }

  String _deckName() {
    var relative = widget.deckPath;
    if (relative.startsWith(widget.rootDir)) {
      relative = relative.substring(widget.rootDir.length);
    }
    final parts = relative
        .split(RegExp(r'[\\/]+'))
        .where((part) => part.isNotEmpty)
        .toList();
    if (parts.length >= 2 && parts[parts.length - 2] == 'decks') {
      parts.removeAt(parts.length - 2);
    }
    return parts.join('/');
  }

  void _openExam() {
    final client = _client;
    final support = _support;
    if (client == null || support == null) return;
    Navigator.of(context).push(
      MaterialPageRoute(
        builder: (_) => ExamScreen(
          deckName: _deckName(),
          client: client,
          support: support,
          buildClient: widget.buildClient ?? HttpServerClient.new,
          applyPassed: (nowMs) => _controller.applyExamPassed(nowMs.toInt()),
          applyFailed: (nowMs) => _controller.applyExamFailed(nowMs.toInt()),
          applyRemediation: (_, _) => 0,
          nowMs: () => BigInt.from(DateTime.now().millisecondsSinceEpoch),
        ),
      ),
    );
  }

  Future<bool> _confirmLeave(BuildContext context) {
    final state = _controller.state;
    return confirmLeaveSession(
      context,
      title: 'Leave the walk?',
      body: "You're on checkpoint ${state.current} of ${state.total}.",
      stayLabel: 'Keep walking',
    );
  }

  @override
  Widget build(BuildContext context) {
    if (_controller.openError != null) {
      return const SizedBox.shrink();
    }
    return ListenableBuilder(
      listenable: _controller,
      builder: (context, _) {
        if (_controller.openError != null) {
          return const SizedBox.shrink();
        }
        final state = _controller.state;
        final done = state.phase == WalkPhaseModel.done;
        final cooldown = done && _client != null
            ? _controller.examCooldownMs(DateTime.now().millisecondsSinceEpoch)
            : null;
        return WalkView(
          state: state,
          predictionController: _predict,
          examAvailable: _controller.serverLive && cooldown == null,
          cooldownMs: cooldown,
          confirmLeave: _confirmLeave,
          onReveal: _submitPredict,
          onGrade: _controller.grade,
          onOpenExam: _openExam,
          onRestart: _restart,
        );
      },
    );
  }
}
