import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge.dart'
    show AnyhowException;
import 'package:path_provider/path_provider.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/exam_screen.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/theme.dart';

const _mono = 'IBM Plex Mono';
const _sans = 'IBM Plex Sans';

/// The on-device trace walk: predict a checkpoint, reveal the real source
/// excerpt, self-grade, repeat, then a done tally that can hand off to the
/// trace exam. Runs entirely on-device (no server call in predict/reveal/
/// grade); only the "Take the exam" handoff needs the paired desktop.
/// Mirrors review_screen.dart's shape (the opaque-session hold, the
/// Scaffold, the phase-based body, `_deckName()`, the exam handoff seam),
/// minus the web's hop rail and live-grade: this walk is always self-graded.
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

  /// The support dir the pairing config is read from; null uses the real
  /// app support dir. Tests inject a temp one.
  final Directory? supportDir;

  /// Builds the exam handoff's client; null uses [HttpServerClient]. Tests
  /// inject a fake, mirroring how ReviewScreen takes its own.
  final ServerClient Function(ServerConfig)? buildClient;

  @override
  State<WalkScreen> createState() => _WalkScreenState();
}

class _WalkScreenState extends State<WalkScreen> {
  late WalkSession _session;
  late WalkState _state;

  /// Why the session could not open (not a trace deck, an unparseable
  /// file). The picker only ever routes trace rows here, so this is a
  /// defensive path, not a normal one: rather than a dedicated "can't open"
  /// surface (ReviewScreen's `_cantOpen`), this bails straight back with a
  /// calm SnackBar.
  String? _openError;

  final TextEditingController _predict = TextEditingController();

  /// Built once a pairing config exists; null when unpaired. Unlike
  /// ReviewScreen's Ask-chip probe, this does not check liveness: the exam
  /// handoff itself (`ExamScreen._start`) surfaces a dead/refused pairing
  /// when opened, so a second probe here would just duplicate that work.
  ServerClient? _client;

  /// Resolved alongside [_client]; handed to [ExamScreen] so its "Re-pair"
  /// action (on a 401 mid-exam) can reopen the pairing sheet, mirroring
  /// review_screen.dart's `_support` cache.
  Directory? _support;

  @override
  void initState() {
    super.initState();
    _open();
    if (_openError != null) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _bailToCaller());
    } else {
      _loadClient();
    }
  }

  @override
  void dispose() {
    _predict.dispose();
    _client?.close();
    super.dispose();
  }

  Future<void> _loadClient() async {
    final support = widget.supportDir ?? await getApplicationSupportDirectory();
    _support = support;
    final config = readServer(support);
    if (config == null || !mounted) return;
    setState(() => _client = (widget.buildClient ?? HttpServerClient.new)(config));
  }

  /// (Re)opens the session; also the "Walk again" action on the done screen.
  void _open() {
    try {
      _session = WalkSession.open(
        deckPath: widget.deckPath,
        rootDir: widget.rootDir,
        device: widget.device,
      );
    } catch (e) {
      _openError = e is AnyhowException ? e.message : '$e';
      return;
    }
    _openError = null;
    _predict.clear();
    _state = _session.state();
  }

  /// Pops back to the caller (the picker) with a calm SnackBar naming why
  /// this deck could not be walked. Captures the messenger before popping:
  /// this screen renders nothing but a blank placeholder while bailing, so
  /// there is no local Scaffold to find one on afterward.
  void _bailToCaller() {
    if (!mounted) return;
    final messenger = ScaffoldMessenger.of(context);
    final message = _openError ?? 'this deck cannot be walked';
    Navigator.of(context).maybePop();
    messenger.showSnackBar(SnackBar(content: Text(message)));
  }

  void _restart() {
    setState(_open);
    if (_openError != null) _bailToCaller();
  }

  void _submitPredict() {
    setState(() {
      _session.predict(text: _predict.text);
      _state = _session.state();
    });
    _predict.clear();
  }

  void _grade(WalkDelta delta) => setState(() => _state = _session.grade(delta: delta));

  /// The name the paired server's own catalog resolves this deck by,
  /// mirroring review_screen.dart's `_deckName()` exactly (see its own
  /// doc): strip the root prefix, then rejoin the remaining path
  /// components with `/`.
  String _deckName() {
    var rel = widget.deckPath;
    if (rel.startsWith(widget.rootDir)) {
      rel = rel.substring(widget.rootDir.length);
    }
    final parts = rel.split(RegExp(r'[\\/]+')).where((p) => p.isNotEmpty);
    return parts.join('/');
  }

  /// Pushes the trace exam. Mirrors `_openExam`'s seam in review_screen.dart:
  /// the screen never touches the bridge itself, only these closures. A
  /// trace never remediates (the server reports `canRemediate: false`), so
  /// `applyRemediation` is a no-op ExamScreen still requires.
  void _openExam(ServerClient client) {
    final support = _support;
    if (support == null) return;
    Navigator.of(context).push(MaterialPageRoute(
      builder: (_) => ExamScreen(
        deckName: _deckName(),
        client: client,
        support: support,
        buildClient: widget.buildClient ?? HttpServerClient.new,
        applyPassed: (nowMs) => _session.applyExamPassed(nowMs: nowMs),
        applyFailed: (nowMs) => _session.applyExamFailed(nowMs: nowMs),
        applyRemediation: (_, _) => 0,
        nowMs: () => BigInt.from(DateTime.now().millisecondsSinceEpoch),
      ),
    ));
  }

  @override
  Widget build(BuildContext context) {
    if (_openError != null) {
      // About to pop (see _bailToCaller); nothing meaningful to render.
      return const SizedBox.shrink();
    }
    final tokens = Theme.of(context).alix;
    final done = _state.phase == WalkPhase.done;
    return Scaffold(
      appBar: AppBar(
        title: const AlixWordmark(),
        actions: [
          if (!done)
            Padding(
              padding: const EdgeInsets.only(right: 16),
              child: Center(
                child: Text(
                  'checkpoint ${_state.current} / ${_state.total}',
                  style: TextStyle(fontFamily: _mono, fontSize: 13, color: tokens.dim),
                ),
              ),
            ),
        ],
      ),
      body: SafeArea(
        child: Column(
          children: [
            if (!done) _descriptionEyebrow(tokens),
            Expanded(
              child: done
                  ? _done(context, tokens)
                  : Column(
                      children: [
                        Expanded(child: _phaseBody(context, tokens)),
                        _footer(context, tokens),
                      ],
                    ),
            ),
          ],
        ),
      ),
    );
  }

  /// The trace's own path description, shown once as a small persistent
  /// header (not restated inline within the prompt/reveal body) so the
  /// learner keeps the "what am I walking" context without it competing
  /// with the checkpoint content.
  Widget _descriptionEyebrow(AlixTokens tokens) {
    if (_state.description.isEmpty) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.fromLTRB(20, 6, 20, 0),
      child: Align(
        alignment: Alignment.centerLeft,
        child: Text(
          _state.description,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(fontFamily: _mono, fontSize: 11.5, letterSpacing: 0.4, color: tokens.bolt),
        ),
      ),
    );
  }

  Widget _phaseBody(BuildContext context, AlixTokens tokens) {
    return SingleChildScrollView(
      padding: const EdgeInsets.fromLTRB(20, 8, 20, 8),
      child: SizedBox(
        width: double.infinity,
        child: _state.phase == WalkPhase.reveal
            ? _revealBody(context, tokens)
            : _predictBody(context, tokens),
      ),
    );
  }

  // ── predict ───────────────────────────────────────────────────────────

  Widget _predictBody(BuildContext context, AlixTokens tokens) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        const SizedBox(height: 8),
        Text(
          _state.prompt ?? '',
          textAlign: TextAlign.center,
          style: TextStyle(
            fontFamily: _sans,
            fontWeight: FontWeight.w600,
            fontSize: 22,
            height: 1.4,
            color: Theme.of(context).colorScheme.onSurface,
          ),
        ),
        _givensRow(tokens),
        _locatorLabel(tokens),
        const SizedBox(height: 20),
        ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 520),
          child: TextField(
            controller: _predict,
            minLines: 3,
            maxLines: 8,
            decoration: InputDecoration(
              filled: true,
              fillColor: Colors.black.withValues(alpha: 0.25),
              border: OutlineInputBorder(borderRadius: BorderRadius.circular(12)),
              hintText: 'predict the next checkpoint, even a hunch beats nothing',
            ),
          ),
        ),
      ],
    );
  }

  Widget _givensRow(AlixTokens tokens) {
    if (_state.givens.isEmpty) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.only(top: 10),
      child: Wrap(
        alignment: WrapAlignment.center,
        spacing: 8,
        runSpacing: 6,
        children: [for (final g in _state.givens) _givenTag(g, tokens)],
      ),
    );
  }

  Widget _givenTag(String text, AlixTokens tokens) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 5),
      decoration: BoxDecoration(
        border: Border.all(color: tokens.line),
        borderRadius: BorderRadius.circular(8),
      ),
      child: Text(text, style: TextStyle(fontFamily: _mono, fontSize: 12, color: tokens.dim)),
    );
  }

  Widget _locatorLabel(AlixTokens tokens) {
    final locator = _state.locator;
    if (locator == null) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.only(top: 8),
      child: Text('at $locator',
          style: TextStyle(fontFamily: _mono, fontSize: 11.5, color: tokens.faint)),
    );
  }

  // ── reveal ────────────────────────────────────────────────────────────

  Widget _revealBody(BuildContext context, AlixTokens tokens) {
    final onSurface = Theme.of(context).colorScheme.onSurface;
    final prediction = _state.prediction;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        _label('you predicted', tokens.dim),
        const SizedBox(height: 4),
        Text(
          prediction ?? '(no prediction)',
          style: TextStyle(
            color: onSurface,
            height: 1.4,
            fontStyle: prediction == null ? FontStyle.italic : FontStyle.normal,
          ),
        ),
        const SizedBox(height: 16),
        _label('the source', tokens.bolt),
        _locatorLabel(tokens),
        const SizedBox(height: 6),
        _excerptBlock(tokens),
        if (_state.points.isNotEmpty) ...[
          const SizedBox(height: 16),
          _pointsList(context, tokens),
        ],
        _noteBlock(tokens),
      ],
    );
  }

  Widget _label(String text, Color color) {
    return Text(
      text.toUpperCase(),
      style: TextStyle(fontFamily: _mono, fontSize: 10.5, letterSpacing: 1.4, color: color),
    );
  }

  /// The revealed source: real gutter-numbered lines when `excerpt` is set,
  /// else the honest `excerptError` rendered calmly (dim, no crash): the
  /// fallback when a checkpoint's `% source:` is a URL or absent.
  Widget _excerptBlock(AlixTokens tokens) {
    final excerpt = _state.excerpt;
    if (excerpt == null) {
      return Container(
        width: double.infinity,
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
        decoration: BoxDecoration(
          border: Border.all(color: tokens.line),
          borderRadius: BorderRadius.circular(10),
        ),
        child: Text(
          _state.excerptError ?? 'no excerpt for this checkpoint',
          style: TextStyle(color: tokens.dim, fontStyle: FontStyle.italic, fontSize: 13),
        ),
      );
    }
    return Container(
      decoration: BoxDecoration(
        border: Border.all(color: tokens.line),
        borderRadius: BorderRadius.circular(10),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 7),
            child: Text(
              excerpt.path,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(fontFamily: _mono, fontSize: 11, color: tokens.dim),
            ),
          ),
          Divider(height: 1, color: tokens.line),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                for (final line in excerpt.lines) _gutterLine(line, tokens),
                if (excerpt.truncated)
                  Padding(
                    padding: const EdgeInsets.only(top: 4),
                    child: Text('… excerpt truncated',
                        style: TextStyle(color: tokens.faint, fontSize: 11, fontStyle: FontStyle.italic)),
                  ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _gutterLine(WalkLine line, AlixTokens tokens) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 1),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 32,
            child: Text('${line.n}', style: TextStyle(fontFamily: _mono, fontSize: 12, color: tokens.faint)),
          ),
          Expanded(
            child: Text(line.text, style: TextStyle(fontFamily: _mono, fontSize: 13, color: tokens.text)),
          ),
        ],
      ),
    );
  }

  Widget _pointsList(BuildContext context, AlixTokens tokens) {
    final onSurface = Theme.of(context).colorScheme.onSurface;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        _label('key points', tokens.good),
        const SizedBox(height: 6),
        for (final pt in _state.points)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 3),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                SizedBox(width: 18, child: Text('▸', style: TextStyle(color: tokens.good, fontSize: 14))),
                Expanded(child: Text(pt, style: TextStyle(color: onSurface, height: 1.4))),
              ],
            ),
          ),
      ],
    );
  }

  /// The connective insight, pinned under the excerpt/points: mirrors
  /// review_screen.dart's `_note` warm box, over a plain string instead of
  /// [NoteUnit]s (a walk's note is one authored sentence, never code).
  Widget _noteBlock(AlixTokens tokens) {
    final note = _state.note;
    if (note == null || note.isEmpty) return const SizedBox.shrink();
    return Container(
      margin: const EdgeInsets.only(top: 16),
      width: double.infinity,
      padding: const EdgeInsets.symmetric(horizontal: 15, vertical: 12),
      decoration: BoxDecoration(
        color: tokens.noteBorder.withValues(alpha: 0.12),
        border: Border.all(color: tokens.noteBorder.withValues(alpha: 0.24)),
        borderRadius: BorderRadius.circular(10),
      ),
      child: Text(note, style: TextStyle(color: tokens.noteInk, fontSize: 15, height: 1.4)),
    );
  }

  // ── footer (predict/reveal actions) ──────────────────────────────────

  Widget _footer(BuildContext context, AlixTokens tokens) {
    final chips = switch (_state.phase) {
      WalkPhase.predict => [_chip('Reveal', _ChipKind.primary, _submitPredict)],
      WalkPhase.reveal => [
          _chip('Missed it', _ChipKind.failed, () => _grade(WalkDelta.missed)),
          _chip('Partly', _ChipKind.partly, () => _grade(WalkDelta.partly)),
          _chip('Got it', _ChipKind.passed, () => _grade(WalkDelta.got)),
        ],
      WalkPhase.done => const <Widget>[],
    };
    if (chips.isEmpty) return SizedBox(height: 12 + MediaQuery.of(context).padding.bottom);
    return Padding(
      padding: EdgeInsets.fromLTRB(12, 10, 12, 12 + MediaQuery.of(context).padding.bottom),
      child: Wrap(alignment: WrapAlignment.center, spacing: 10, runSpacing: 8, children: chips),
    );
  }

  // ── done ──────────────────────────────────────────────────────────────

  Widget _done(BuildContext context, AlixTokens tokens) {
    final summary =
        _state.summary ?? WalkSummary(passed: 0, partly: 0, failed: 0, weak: Uint32List(0), total: 0);
    final client = _client;
    final cooldown = client == null
        ? null
        : _session.examCooldownMs(nowMs: BigInt.from(DateTime.now().millisecondsSinceEpoch));
    final examAvailable = client != null && cooldown == null;
    return SingleChildScrollView(
      padding: const EdgeInsets.all(24),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 12),
          Text(
            'WALK COMPLETE',
            style: TextStyle(fontFamily: _mono, color: tokens.bolt, fontSize: 11, letterSpacing: 2.2),
          ),
          const SizedBox(height: 14),
          Text(
            _state.description.isEmpty ? 'Trace walked.' : _state.description,
            style: TextStyle(
                fontSize: 24, fontWeight: FontWeight.w600, color: Theme.of(context).colorScheme.onSurface),
          ),
          const SizedBox(height: 18),
          _summaryRow('got it', '${summary.passed}', tokens, valueColor: tokens.good),
          _summaryRow('partly', '${summary.partly}', tokens, valueColor: tokens.warn),
          _summaryRow('missed it', '${summary.failed}', tokens, valueColor: tokens.again),
          if (summary.weak.isNotEmpty)
            _summaryRow('weak (resurface sooner)', summary.weak.map((h) => '#$h').join(' · '), tokens)
          else if (summary.total > 0)
            _summaryRow('every checkpoint landed', '✓', tokens, valueColor: tokens.good),
          const SizedBox(height: 24),
          if (examAvailable) ...[
            _chip('Take the exam', _ChipKind.primary, () => _openExam(client)),
            const SizedBox(height: 12),
          ],
          _chip('Walk again', examAvailable ? _ChipKind.base : _ChipKind.primary, _restart),
          if (client != null && cooldown != null) ...[
            const SizedBox(height: 12),
            Text(
              'Walk the trace again before re-sitting; ${_humanizeCooldown(cooldown)} left.',
              style: TextStyle(color: tokens.dim, fontSize: 13),
            ),
          ],
        ],
      ),
    );
  }

  Widget _summaryRow(String label, String value, AlixTokens tokens, {Color? valueColor}) {
    return Container(
      padding: const EdgeInsets.symmetric(vertical: 9),
      decoration: BoxDecoration(border: Border(bottom: BorderSide(color: tokens.line))),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Text(label, style: TextStyle(color: tokens.dim)),
          Text(
            value,
            style: TextStyle(
                fontFamily: _mono, fontWeight: FontWeight.w600, color: valueColor ?? Theme.of(context).colorScheme.onSurface),
          ),
        ],
      ),
    );
  }

  /// Ceils to whole minutes so a cooldown with seconds left never reads
  /// "0m" (the default cooldown is an hour, so this is coarse on purpose).
  String _humanizeCooldown(BigInt ms) {
    final minutes = (ms.toInt() / 60000).ceil();
    return minutes <= 1 ? 'about a minute' : 'about ${minutes}m';
  }

  // ── chips ─────────────────────────────────────────────────────────────

  Widget _chip(String label, _ChipKind kind, VoidCallback? onTap) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    Color? fill;
    Color borderColor = Colors.transparent;
    Color fg;
    switch (kind) {
      case _ChipKind.base:
        borderColor = tokens.line;
        fg = tokens.text;
      case _ChipKind.primary:
        fill = theme.colorScheme.primary;
        borderColor = theme.colorScheme.primary;
        fg = theme.colorScheme.onPrimary;
      case _ChipKind.failed:
        fill = tokens.again.withValues(alpha: 0.12);
        borderColor = tokens.again.withValues(alpha: 0.42);
        fg = tokens.again;
      case _ChipKind.partly:
        fill = tokens.warn.withValues(alpha: 0.14);
        borderColor = tokens.warn.withValues(alpha: 0.42);
        fg = tokens.warn;
      case _ChipKind.passed:
        fill = tokens.good.withValues(alpha: 0.13);
        borderColor = tokens.good.withValues(alpha: 0.42);
        fg = tokens.good;
    }
    return Material(
      color: fill ?? Colors.transparent,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(10),
        side: BorderSide(color: borderColor),
      ),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(10),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 15, vertical: 11),
          child: Text(
            label,
            style: TextStyle(fontFamily: _sans, fontWeight: FontWeight.w600, fontSize: 14, color: fg),
          ),
        ),
      ),
    );
  }
}

enum _ChipKind { base, primary, failed, partly, passed }
