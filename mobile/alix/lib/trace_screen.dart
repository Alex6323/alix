import 'package:flutter/material.dart';

import 'package:alix_mobile/bridge/trace_bridge.dart';
import 'package:alix_mobile/leave_guard.dart';
import 'package:alix_mobile/sync/sync_controller.dart';
import 'package:alix_mobile/sync/sync_sheet.dart';
import 'package:alix_mobile/trace/trace_controller.dart';
import 'package:alix_mobile/trace/trace_models.dart';
import 'package:alix_mobile/trace/trace_view.dart';

/// The on-device trace route.
///
/// The screen owns Flutter lifecycle and navigation.
/// Bridge-backed session state and named transitions live in [TraceSessionController],
/// while the rendered tree lives below [TraceSessionView].
class TraceSessionScreen extends StatefulWidget {
  const TraceSessionScreen({
    super.key,
    required this.deckPath,
    required this.rootDir,
    this.device,
    this.syncController,
  });

  final String deckPath;
  final String rootDir;

  /// This install's label for the store's last-writer marker.
  final String? device;

  /// Non-null only when [rootDir] is the active paired root.
  final SyncController? syncController;

  @override
  State<TraceSessionScreen> createState() => _TraceSessionScreenState();
}

class _TraceSessionScreenState extends State<TraceSessionScreen> {
  late final TraceSessionController _controller;
  final TextEditingController _predict = TextEditingController();

  bool _endChecked = false;

  @override
  void initState() {
    super.initState();
    _controller = TraceSessionController(
      factory: const TraceSessionBridgeFactory(),
      deckPath: widget.deckPath,
      rootDir: widget.rootDir,
      device: widget.device,
    );
    if (_controller.openError != null) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _bailToCaller());
    } else {
      _controller.addListener(_checkConflictAtDone);
    }
  }

  void _checkConflictAtDone() {
    if (_endChecked ||
        _controller.openError != null ||
        _controller.state.phase != TraceSessionPhaseModel.done) {
      return;
    }
    _endChecked = true;
    _showEndConflict();
  }

  Future<void> _showEndConflict() async {
    final syncController = widget.syncController;
    if (syncController == null || !mounted) return;
    await showSessionEndConflict(
      context,
      syncController: syncController,
      rootDir: widget.rootDir,
      deckPath: widget.deckPath,
    );
  }

  @override
  void dispose() {
    _predict.dispose();
    _controller.removeListener(_checkConflictAtDone);
    _controller.dispose();
    super.dispose();
  }

  void _bailToCaller() {
    if (!mounted) return;
    final messenger = ScaffoldMessenger.of(context);
    final message = _controller.openError ?? 'this deck cannot be traced';
    Navigator.of(context).maybePop();
    messenger.showSnackBar(SnackBar(content: Text(message)));
  }

  void _restart() {
    _endChecked = false;
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

  Future<bool> _confirmLeave(BuildContext context) async {
    final state = _controller.state;
    final leave = await confirmLeaveSession(
      context,
      title: 'Leave the trace?',
      body: "You're on checkpoint ${state.current} of ${state.total}.",
      stayLabel: 'Keep tracing',
    );
    if (leave) await _showEndConflict();
    return leave;
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
        return TraceSessionView(
          state: state,
          predictionController: _predict,
          confirmLeave: _confirmLeave,
          onReveal: _submitPredict,
          onGrade: _controller.grade,
          onRestart: _restart,
        );
      },
    );
  }
}
