import 'dart:io';

import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/bridge/review_bridge.dart';
import 'package:alix_mobile/exam_screen.dart';
import 'package:alix_mobile/leave_guard.dart';
import 'package:alix_mobile/pairing_sheet.dart';
import 'package:alix_mobile/review/review_controller.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/review_view.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/sync/sync_controller.dart';
import 'package:alix_mobile/sync/sync_models.dart';
import 'package:alix_mobile/sync/sync_sheet.dart';
import 'package:alix_mobile/tutor_sheet.dart';

class ReviewScreen extends StatefulWidget {
  const ReviewScreen({
    super.key,
    required this.deckPath,
    required this.rootDir,
    required this.depth,
    this.device,
    this.supportDir,
    this.buildClient,
    this.syncController,
  });

  final String deckPath;
  final String rootDir;

  /// Null lets the core resolve the remembered depth or the deck default.
  final ReviewDepth? depth;

  /// This install's label for the store's last-writer marker.
  final String? device;

  /// The support dir the pairing config is read from. Tests inject a temp one.
  final Directory? supportDir;

  /// Builds the server probe's client. Tests inject a fake.
  final ServerClient Function(ServerConfig)? buildClient;

  /// Non-null only when [rootDir] is the active paired root: once the
  /// summary renders, this deck's local progress is pushed silently.
  final SyncController? syncController;

  @override
  State<ReviewScreen> createState() => _ReviewScreenState();
}

class _ReviewScreenState extends State<ReviewScreen> {
  late final ReviewController _controller;
  final List<TextEditingController> _typed = [];
  final TextEditingController _attempt = TextEditingController();

  ServerClient? _client;
  Directory? _support;
  bool _summaryPushed = false;

  @override
  void initState() {
    super.initState();
    _controller = ReviewController(
      factory: const ReviewBridgeFactory(),
      deckPath: widget.deckPath,
      rootDir: widget.rootDir,
      depth: widget.depth,
      device: widget.device,
    );
    if (_controller.openError == null) {
      _resetInputs();
      _probeServer();
      _surfaceLoadWarnings();
      _controller.addListener(_maybePushSummary);
    }
  }

  /// Fires once, the first time the summary renders (`state.card` turns
  /// null), pushing this deck's local progress silently; a 409 opens the
  /// same conflict choice the sync report sheet uses.
  void _maybePushSummary() {
    if (_summaryPushed || _controller.state.card != null) return;
    final syncController = widget.syncController;
    if (syncController == null) return;
    final deckId = deckIdForPath(
      entries: syncController.pairedEntries,
      rootDir: widget.rootDir,
      path: widget.deckPath,
    );
    if (deckId == null) return;
    _summaryPushed = true;
    syncController
        .pushOne(deckId)
        .then((_) {
          if (!mounted) return;
          final conflicts = syncController.pendingConflicts.where(
            (c) => c.deckId == deckId,
          );
          if (conflicts.isNotEmpty) _showConflictSheet(conflicts.first);
        })
        // Silent on failure, matching the doc comment above: an unexpected
        // error here must not become an unhandled Future error (a crash
        // report the user never sees a symptom for).
        .catchError((_) {});
  }

  void _showConflictSheet(SyncPendingConflict conflict) {
    final syncController = widget.syncController;
    if (syncController == null || !mounted) return;
    showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      builder: (sheet) => SyncReportSheet(
        report: null,
        conflicts: [conflict],
        onResolve: (deckId, keepPhone) {
          syncController.resolve(deckId, keepPhone: keepPhone);
          Navigator.of(sheet).pop();
        },
        onRemoveOrphan: (_) {},
      ),
    );
  }

  // One transient line at open (the web client's notice semantics): the
  // deck still reviews, but a fallback must not pass for a frozen diagram.
  void _surfaceLoadWarnings() {
    final warnings = _controller.state.loadWarnings;
    if (warnings.isEmpty) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(SnackBar(content: Text(warnings.join(' '))));
    });
  }

  @override
  void dispose() {
    for (final controller in _typed) {
      controller.dispose();
    }
    _attempt.dispose();
    _controller.removeListener(_maybePushSummary);
    _controller.dispose();
    _client?.close();
    super.dispose();
  }

  Future<void> _probeServer() async {
    final support = widget.supportDir ?? await getApplicationSupportDirectory();
    _support = support;
    final config = readActivePairing(support);
    if (config == null) return;
    final client = (widget.buildClient ?? HttpServerClient.new)(config);
    ServerVersion? probe;
    try {
      probe = await client.version();
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
        probe != null && compareVersions(probe.version, minServerVersion) >= 0;
    if (!live || !mounted) {
      client.close();
      return;
    }
    _client = client;
    _controller.setServerLive(true);
  }

  void _openTutor(ReviewTutorCardModel tutor) {
    final client = _client;
    if (client == null) return;
    showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      builder: (_) => TutorSheet(
        card: TutorCardContext(
          subject: tutor.subject,
          front: tutor.front,
          back: tutor.back,
          at: tutor.at,
        ),
        client: client,
        mint: (front, back) async => _controller.mintTutorCard(
          front: front,
          back: back,
          nowMs: DateTime.now().millisecondsSinceEpoch,
        ),
        onNote: (notes) =>
            _controller.applyCardNote(id: tutor.id, notes: notes),
      ),
    );
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
          applyRemediation: (cardsText, nowMs) => _controller.applyRemediation(
            cardsText: cardsText,
            nowMs: nowMs.toInt(),
          ),
          nowMs: () => BigInt.from(DateTime.now().millisecondsSinceEpoch),
        ),
      ),
    );
  }

  void _resetInputs() {
    final state = _controller.state;
    final lines = state.card?.gradeableSteps.length ?? 1;
    while (_typed.length < lines) {
      _typed.add(TextEditingController());
    }
    for (final controller in _typed) {
      controller.clear();
    }
    _attempt.clear();
  }

  void _introduce() {
    _controller.introduce();
    _resetInputs();
  }

  void _grade(ReviewGrade grade) {
    _controller.grade(grade);
    _resetInputs();
  }

  void _restart() {
    _controller.restart();
    if (_controller.openError == null) _resetInputs();
  }

  Future<bool> _confirmLeave(BuildContext context) {
    final remaining = _controller.state.remaining;
    return confirmLeaveSession(
      context,
      title: 'Leave the review?',
      body:
          '$remaining card${remaining == 1 ? '' : 's'} still due in this session.',
      stayLabel: 'Keep reviewing',
    );
  }

  @override
  Widget build(BuildContext context) {
    if (_controller.openError case final error?) {
      return ReviewCantOpenView(message: error);
    }
    return ListenableBuilder(
      listenable: _controller,
      builder: (context, _) {
        if (_controller.openError case final error?) {
          return ReviewCantOpenView(message: error);
        }
        final state = _controller.state;
        final card = state.card;
        final nowMs = DateTime.now().millisecondsSinceEpoch;
        final explain =
            state.mode == ReviewMode.explain &&
            !state.introducing &&
            state.keypoints != null;
        return ReviewView(
          state: state,
          crumb: _controller.crumb(nowMs),
          revealed: _controller.revealed,
          revealedLines: _controller.revealedLines,
          choice: _controller.choice,
          multiChoice: _controller.multiChoice,
          multiSelected: _controller.multiSelected,
          checkFeedback: _controller.checkFeedback,
          typelineChecked: _controller.typelineChecked,
          tickedKeypoints: _controller.tickedKeypoints,
          sketch: _controller.sketch,
          onSketchBegin: _controller.sketchBegin,
          onSketchExtend: _controller.sketchExtend,
          onSketchEnd: _controller.sketchEnd,
          onSketchTool: _controller.selectSketchTool,
          onSketchUndo: _controller.sketchUndo,
          onSketchClear: _controller.sketchClear,
          attemptOpen: _controller.attemptOpen,
          attemptController: _attempt,
          typedControllers: _typed,
          serverLive: _controller.serverLive,
          tutorCard: card == null ? null : _controller.tutorCard,
          verdictGrade: explain ? _controller.verdictGrade : ReviewGrade.pass,
          examAvailable:
              card == null && _controller.serverLive && _controller.deckHasExam,
          nowMs: nowMs,
          confirmLeave: _confirmLeave,
          onChoose: _controller.choose,
          onToggleChoice: _controller.toggleChoice,
          onSubmitChoices: _controller.submitChoices,
          onCheck: (lines) {
            final before = _controller.typelineChecked.length;
            _controller.check(lines);
            // TypeLine reuses the one open field, so it is emptied only when
            // the server actually accepted a longer prefix: a rejected or
            // failed check leaves the learner's text where they can see it.
            if (_controller.typelineChecked.length > before) _typed[0].clear();
          },
          onOpenAttempt: _controller.openAttempt,
          onToggleKeypoint: _controller.toggleKeypoint,
          onReveal: _controller.reveal,
          onRevealNextLine: _controller.revealNextLine,
          onIntroduce: _introduce,
          onGrade: _grade,
          onOpenTutor: _openTutor,
          onRestart: _restart,
          onOpenExam: _openExam,
        );
      },
    );
  }
}
