// The AI exam, run on the phone against a paired desktop: it generates
// questions and grades against its own deck copy while every outcome
// (the sitting's result, remediation cards) lands in the PHONE's store via
// the bridge callbacks picker_screen.dart hands in. Data and callbacks only: this file
// never imports the generated bridge (`src/rust/*`), so its tests run
// without a Rust dylib, the same seam tutor_sheet.dart plays.
import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';

import 'package:alix_mobile/pairing_sheet.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/theme.dart';

/// The name the paired server's own catalog resolves a deck by: a bare
/// top-level file name, or `<workspace>/<file>` for a workspace member.
/// Never a device-absolute path.
String examDeckName({required String deckPath, required String rootDir}) {
  var relative = deckPath;
  if (relative.startsWith(rootDir)) {
    relative = relative.substring(rootDir.length);
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

/// The exact wording for a 401 at any phase: the paired server is right
/// there but rejects this app's token. Every [ServerClient] call can throw
/// [PairingExpired]; every call site here catches it, shows exactly this
/// line, and pops (there is nothing left to sit an exam against).
const _pairingExpiredMessage =
    'Pairing expired. Pair again from Settings → Connected devices.';

/// Phases `GET /api/remote/exam` cycles through while doing async work: the
/// screen keeps polling for as long as the last DTO reports one of these
/// (or `thinking`); `answering` is client-driven (no poll needed) and
/// `results`/`remediated`/`error` are terminal for their stage.
const _pollingPhases = {'generating', 'grading', 'remediating'};

/// A full-screen AI exam sitting on the paired desktop: one immersive flow,
/// mirroring how review_screen.dart structures itself (a plain header, the
/// deck name truncating, a Close affordance). Phase-driven off
/// [RemoteExam.phase]; see `docs/API.md` 4.10 / `RemoteExamDto`.
class ExamScreen extends StatefulWidget {
  const ExamScreen({
    super.key,
    required this.deckName,
    required this.client,
    required this.support,
    required this.buildClient,
    required this.applyResult,
    required this.applyRemediation,
    required this.nowMs,
    this.cards,
    this.initialExam,
    this.ideas,
    this.loadAnswers,
    this.saveAnswers,
    this.clearAnswers,
    this.saveIdeas,
    this.pollInterval = const Duration(milliseconds: 400),
  });

  /// The name the paired server's own catalog resolves this deck by: a bare
  /// top-level file name, or `<workspace>/<file>` for a workspace member.
  /// Never a device-absolute path; see [examDeckName].
  final String deckName;

  /// The paired desktop's AI backend, over `/api/remote/*`.
  final ServerClient client;

  /// The support dir the "Re-pair" action reopens the pairing sheet
  /// against on a 401 (see `_expirePairingAndPop`); the caller (review or
  /// trace screen) resolves this once and hands it down.
  final Directory support;

  /// Builds the pairing sheet's own probe client, the same way the caller
  /// resolves its own.
  final ServerClient Function(ServerConfig) buildClient;

  /// Records a settled sitting, whatever its outcome, in the phone's own
  /// store (a closure over `applyExamResult`). Called at most once per
  /// sitting, with the terminal DTO.
  final String? Function(RemoteExam exam, BigInt nowMs) applyResult;

  /// The card ids whose topics to examine; null sits the full deck.
  final List<String>? cards;

  final RemoteExam? initialExam;

  final Map<String, dynamic>? ideas;

  final List<String> Function(List<String> questions)? loadAnswers;

  final void Function(List<String> questions, List<String> answers)? saveAnswers;

  final VoidCallback? clearAnswers;

  final void Function(Map<String, dynamic> ideas)? saveIdeas;

  /// Turns one failed topic's remediation deck-text into phone-store
  /// personal cards bound to that topic (a closure over
  /// `applyRemediation`), returning how many were created or revived.
  final int Function(String topicId, String cardsText) applyRemediation;

  /// The wall clock, injected so tests can fake it.
  final BigInt Function() nowMs;

  /// How often to poll `GET /api/remote/exam` while the server is working.
  /// Tests shrink this well below the default.
  final Duration pollInterval;

  @override
  State<ExamScreen> createState() => _ExamScreenState();
}

class _ExamScreenState extends State<ExamScreen> {
  RemoteExam? _exam;
  Timer? _pollTimer;

  int _currentQuestion = 0;
  final List<TextEditingController> _answerControllers = [];
  final List<bool> _answerLocked = [];
  bool _submitting = false;
  bool _remediating = false;
  bool _acceptedStart = false;
  bool _explicitlyClosed = false;
  bool _closing = false;

  /// Guards against a sitting recording its result more than once: a stray
  /// extra poll landing on the same terminal `results` DTO (e.g. two
  /// periodic ticks resolving out of order) must not double-apply.
  bool _resultApplied = false;
  String? _standing;

  /// Same guard, for turning `remediated` cards into store entries.
  bool _remediationApplied = false;

  /// How many consecutive null first-poll replies to tolerate before
  /// falling back to the refusal path (Polish 8b): a slow sitting start
  /// must not strand the screen, but a genuinely dead server must not spin
  /// forever either.
  static const _firstPollRetryLimit = 4;
  int _firstPollAttempts = 0;

  @override
  void initState() {
    super.initState();
    final initialExam = widget.initialExam;
    if (initialExam == null) {
      _start();
    } else {
      _acceptedStart = true;
      scheduleMicrotask(() => _applyDto(initialExam));
    }
  }

  bool get _keepsSitting {
    final phase = _exam?.phase;
    return phase == 'generating' ||
        phase == 'answering' ||
        (_acceptedStart && phase == null);
  }

  @override
  void dispose() {
    _pollTimer?.cancel();
    final leaving = !_explicitlyClosed && !_keepsSitting
        ? widget.client.examClose()
        : Future<void>.value();
    leaving.catchError((_) {}).whenComplete(widget.client.close);
    for (final c in _answerControllers) {
      c.dispose();
    }
    super.dispose();
  }

  // ── the phase walk ───────────────────────────────────────────────────

  Future<void> _start() async {
    _acceptedStart = true;
    bool ok;
    try {
      ok = await widget.client.examStart(
        widget.deckName,
        cards: widget.cards,
        ideas: widget.ideas,
      );
    } on PairingExpired {
      _expirePairingAndPop();
      return;
    }
    if (!mounted) return;
    if (!ok) {
      _snackAndPop('The desktop refused the exam.');
      return;
    }
    await _poll();
  }

  Future<void> _poll() async {
    RemoteExam? dto;
    try {
      dto = await widget.client.examGet();
    } on PairingExpired {
      _pollTimer?.cancel();
      _expirePairingAndPop();
      return;
    }
    if (!mounted) return;
    if (dto == null) {
      _retryFirstPollOrGiveUp();
      return;
    }
    _applyDto(dto);
  }

  /// Polish 8b: the very first poll can return null before the sitting is
  /// ready on the server (a slow start), which would otherwise strand the
  /// screen on a blank state forever (no timer is running yet to retry
  /// it). Once a real DTO has ever landed (`_exam != null`), a later null
  /// is left to the already-running periodic timer, which retries on its
  /// own. A bounded number of retries recovers a slow start without
  /// turning a genuinely dead server into an infinite spinner.
  void _retryFirstPollOrGiveUp() {
    if (_exam != null) return;
    _firstPollAttempts++;
    if (_firstPollAttempts > _firstPollRetryLimit) {
      _snackAndPop('The desktop refused the exam.');
      return;
    }
    if (!mounted) return;
    _pollTimer?.cancel();
    _pollTimer = Timer(widget.pollInterval, () => _poll());
  }

  /// The one place a fresh DTO lands: applies its one-shot side effects
  /// (guarded, so a duplicate delivery is a no-op), builds the answer
  /// controllers the first time `answering` is seen, renders, then decides
  /// whether to keep polling.
  void _applyDto(RemoteExam dto) {
    if (!mounted) return;
    if (dto.phase == 'results' && !_resultApplied) {
      final ideas = dto.ideas;
      if (ideas != null) widget.saveIdeas?.call(ideas);
      _standing = widget.applyResult(dto, widget.nowMs());
      _resultApplied = true;
      widget.clearAnswers?.call();
    }
    if (dto.phase == 'remediated' && !_remediationApplied) {
      _remediationApplied = true;
      var count = 0;
      for (final topic in dto.cardsByTopic) {
        count += widget.applyRemediation(topic.topicId, topic.cards);
      }
      _snack('$count new cards to drill.');
    }
    setState(() {
      _exam = dto;
      if (dto.phase == 'answering' && _answerControllers.isEmpty) {
        _installAnswers(dto.questions);
      }
    });
    _rescheduleTimer(dto);
  }

  void _installAnswers(List<String> questions) {
    final loaded = widget.loadAnswers?.call(questions);
    final answers = loaded != null && loaded.length == questions.length
        ? loaded
        : List.filled(questions.length, '');
    _answerControllers.addAll([
      for (final answer in answers) TextEditingController(text: answer),
    ]);
    _answerLocked.addAll([for (final answer in answers) answer.isNotEmpty]);
    final firstEmpty = answers.indexWhere((answer) => answer.isEmpty);
    _currentQuestion = firstEmpty >= 0
        ? firstEmpty
        : (questions.isEmpty ? 0 : questions.length - 1);
  }

  bool _lockAndPersistCurrent() {
    if (_answerControllers.isEmpty) return true;
    final index = _currentQuestion;
    if (_answerLocked[index] || _answerControllers[index].text.trim().isEmpty) {
      return true;
    }
    _answerLocked[index] = true;
    try {
      widget.saveAnswers?.call(
        _exam!.questions,
        [for (final controller in _answerControllers) controller.text],
      );
    } on Object catch (error) {
      _answerLocked[index] = false;
      _snack('Could not save the answer: $error');
      return false;
    }
    return true;
  }

  void _nextQuestion() {
    if (!_lockAndPersistCurrent()) return;
    setState(() => _currentQuestion++);
  }

  /// Every timer (re)start is mounted-guarded: a dismissed screen must
  /// never start a poll that outlives it.
  void _rescheduleTimer(RemoteExam dto) {
    _pollTimer?.cancel();
    final keepPolling =
        dto.error == null && (dto.thinking || _pollingPhases.contains(dto.phase));
    if (!keepPolling || !mounted) return;
    _pollTimer = Timer.periodic(widget.pollInterval, (_) => _poll());
  }

  Future<void> _submit() async {
    if (_submitting) return;
    if (!_lockAndPersistCurrent()) return;
    final answers = [for (final c in _answerControllers) c.text];
    setState(() => _submitting = true);
    bool ok;
    try {
      ok = await widget.client.examGrade(answers);
    } on PairingExpired {
      _expirePairingAndPop();
      return;
    }
    if (!mounted) return;
    setState(() => _submitting = false);
    if (!ok) {
      _snack('Could not submit. Try again.');
      return;
    }
    await _poll();
  }

  Future<void> _explicitClose() async {
    if (_closing) return;
    setState(() {
      _closing = true;
      _explicitlyClosed = true;
    });
    _pollTimer?.cancel();
    try {
      await widget.client.examClose();
    } on Object catch (_) {}
    widget.clearAnswers?.call();
    if (!mounted) return;
    final navigator = Navigator.of(context);
    if (navigator.canPop()) navigator.pop();
  }

  Future<void> _remediate() async {
    if (_remediating) return;
    setState(() => _remediating = true);
    bool ok;
    try {
      ok = await widget.client.examRemediate();
    } on PairingExpired {
      _expirePairingAndPop();
      return;
    }
    if (!mounted) return;
    setState(() => _remediating = false);
    if (!ok) {
      _snack('Could not remediate. Try again.');
      return;
    }
    await _poll();
  }

  // ── shared ────────────────────────────────────────────────────────────

  void _expirePairingAndPop() {
    _pollTimer?.cancel();
    if (!mounted) return;
    // This screen's own context dies the moment it pops below, but a
    // "Re-pair" tap on the SnackBar (shown on the ROOT messenger, so it
    // survives) can fire well after that. Capture the navigator itself
    // (not this screen's context) now: it outlives the pop, and `.mounted`
    // at tap time guards the one case it wouldn't (the whole app tearing
    // down in between).
    final navigator = Navigator.of(context);
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(
      content: const Text(_pairingExpiredMessage),
      action: SnackBarAction(
        label: 'Re-pair',
        onPressed: () {
          if (!navigator.mounted) return;
          showPairingSheet(navigator.context, support: widget.support, buildClient: widget.buildClient);
        },
      ),
    ));
    navigator.maybePop();
  }

  void _snackAndPop(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(message)));
    Navigator.of(context).maybePop();
  }

  void _snack(String text) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(text)));
  }

  // ── build ─────────────────────────────────────────────────────────────

  @override
  Widget build(BuildContext context) {
    return PopScope(
      canPop: _keepsSitting && !_closing,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) _explicitClose();
      },
      child: Scaffold(
        appBar: AppBar(
          automaticallyImplyLeading: false,
          leading: IconButton(
            icon: const Icon(Icons.close),
            onPressed: () => Navigator.of(context).maybePop(),
          ),
          title: Text(widget.deckName, maxLines: 1, overflow: TextOverflow.ellipsis),
        ),
        body: SafeArea(
          child: Padding(
            padding: const EdgeInsets.all(20),
            child: _body(context),
          ),
        ),
      ),
    );
  }

  Widget _body(BuildContext context) {
    final exam = _exam;
    final error = exam?.error;
    if (error != null) return _errorState(context, error);
    final phase = exam?.phase;
    if (phase == null || _pollingPhases.contains(phase)) {
      return _working(context, exam?.elapsed, phase);
    }
    switch (phase) {
      case 'answering':
        return _answering(context, exam!);
      case 'results':
        return _results(context, exam!);
      case 'remediated':
        return _remediatedDone(context);
      default:
        // Open phase vocabulary (docs/API.md): an unrecognized phase reads
        // as still working rather than a dead end.
        return _working(context, exam?.elapsed, phase);
    }
  }

  Widget _working(BuildContext context, int? elapsed, String? phase) {
    final tokens = Theme.of(context).alix;
    final suffix = elapsed != null ? ' ${elapsed}s' : '';
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(
            'The server is working…$suffix',
            // Matches the web's `.exam-wait` idiom (dim + italic).
            style: TextStyle(color: tokens.dim, fontStyle: FontStyle.italic),
          ),
          if (phase == 'generating') ...[
            const SizedBox(height: 16),
            OutlinedButton(
              onPressed: _closing ? null : _explicitClose,
              child: const Text('Cancel preparation'),
            ),
          ],
        ],
      ),
    );
  }

  Widget _errorState(BuildContext context, String error) {
    final tokens = Theme.of(context).alix;
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          // Matches the web's `.exam-error` color token (red, not neutral).
          Text('The exam backend failed.', style: TextStyle(color: tokens.again)),
          const SizedBox(height: 8),
          Text(error, textAlign: TextAlign.center, style: TextStyle(color: tokens.dim)),
          const SizedBox(height: 16),
          OutlinedButton(
            onPressed: _explicitClose,
            child: const Text('Close'),
          ),
        ],
      ),
    );
  }

  Widget _answering(BuildContext context, RemoteExam exam) {
    final tokens = Theme.of(context).alix;
    final total = exam.questions.length;
    final index = _currentQuestion;
    final last = index == total - 1;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text('Question ${index + 1} of $total',
            style: TextStyle(color: tokens.dim, fontSize: 13)),
        const SizedBox(height: 12),
        Text(exam.questions[index], style: Theme.of(context).textTheme.titleMedium),
        const SizedBox(height: 16),
        Expanded(
          child: TextField(
            key: const ValueKey('exam-answer-field'),
            controller: _answerControllers[index],
            readOnly: _answerLocked[index],
            maxLines: null,
            expands: true,
            textAlignVertical: TextAlignVertical.top,
            decoration: const InputDecoration(hintText: 'your answer'),
          ),
        ),
        const SizedBox(height: 12),
        Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            OutlinedButton(
              onPressed: index > 0 ? () => setState(() => _currentQuestion--) : null,
              child: const Text('Back'),
            ),
            FilledButton(
              onPressed: last
                  ? (_submitting ? null : _submit)
                  : _nextQuestion,
              child: Text(last ? 'Submit' : 'Next'),
            ),
          ],
        ),
        TextButton(
          onPressed: _closing ? null : _explicitClose,
          child: const Text('End exam'),
        ),
      ],
    );
  }

  Widget _results(BuildContext context, RemoteExam exam) {
    final tokens = Theme.of(context).alix;
    final verdicts = exam.verdicts;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (verdicts != null)
          Text(
            '${_points(verdicts.points)} / ${verdicts.total}',
            style: TextStyle(
              fontSize: 22,
              fontWeight: FontWeight.w600,
              color: tokens.text,
            ),
          ),
        if (_standing case final standing?)
          Text(standing, style: TextStyle(color: tokens.good, fontSize: 13)),
        const SizedBox(height: 16),
        Expanded(
          child: ListView(
            children: [
              for (final topic in exam.topics) _topicRow(topic, tokens),
              if (exam.topics.isNotEmpty) const SizedBox(height: 8),
              for (final g in exam.grades) _gradeRow(g, tokens),
              if (exam.gaps.isNotEmpty) ...[
                const SizedBox(height: 12),
                Text('gaps', style: TextStyle(color: tokens.faint, fontSize: 11, letterSpacing: 1.4)),
                for (final gap in exam.gaps)
                  Padding(
                    padding: const EdgeInsets.symmetric(vertical: 2),
                    child: Text('• $gap', style: TextStyle(color: tokens.dim)),
                  ),
              ],
            ],
          ),
        ),
        if (exam.canRemediate) ...[
          const SizedBox(height: 12),
          FilledButton(
            onPressed: _remediating ? null : _remediate,
            child: const Text('Turn the gaps into cards'),
          ),
        ],
      ],
    );
  }

  String _points(double points) =>
      points == points.roundToDouble() ? '${points.toInt()}' : points.toStringAsFixed(1);

  Widget _topicRow(RemoteExamTopic topic, AlixTokens tokens) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        children: [
          Expanded(
            child: Text(topic.label, maxLines: 1, overflow: TextOverflow.ellipsis),
          ),
          const SizedBox(width: 12),
          Text(
            '${_points(topic.verdicts.points)} / ${topic.verdicts.total}',
            style: TextStyle(color: tokens.dim),
          ),
          const SizedBox(width: 12),
          Text(
            topic.pass ? 'pass' : 'needs work',
            style: TextStyle(color: topic.pass ? tokens.good : tokens.again, fontSize: 12),
          ),
        ],
      ),
    );
  }

  Widget _gradeRow(RemoteExamGrade g, AlixTokens tokens) {
    final color = switch (g.verdict) {
      'PASS' => tokens.good,
      'PARTIAL' => tokens.warn,
      _ => tokens.again,
    };
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Text(g.verdict, style: TextStyle(fontWeight: FontWeight.w600, color: color)),
              if (g.shape.isNotEmpty) ...[
                const SizedBox(width: 8),
                Text(g.shape, style: TextStyle(color: tokens.faint, fontSize: 11)),
              ],
            ],
          ),
          const SizedBox(height: 4),
          Text(g.feedback, style: TextStyle(color: tokens.dim)),
        ],
      ),
    );
  }

  Widget _remediatedDone(BuildContext context) {
    final tokens = Theme.of(context).alix;
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text('Done.', style: TextStyle(fontSize: 22, fontWeight: FontWeight.w600, color: tokens.text)),
          const SizedBox(height: 16),
          FilledButton(
            onPressed: _explicitClose,
            child: const Text('Close'),
          ),
        ],
      ),
    );
  }
}
