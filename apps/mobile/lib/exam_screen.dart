// The AI exam, run on the phone against a paired desktop: it generates
// questions and grades against its own deck copy while every outcome
// (mastery, remediation cards) lands in the PHONE's store via the bridge
// callbacks review_screen.dart hands in. Data and callbacks only: this file
// never imports the generated bridge (`src/rust/*`), so its tests run
// without a Rust dylib, the same seam tutor_sheet.dart plays.
import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';

import 'package:alix_mobile/pairing_sheet.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/theme.dart';

/// The exact wording for a 401 at any phase: the paired server is right
/// there but rejects this app's token. Every [ServerClient] call can throw
/// [PairingExpired]; every call site here catches it, shows exactly this
/// line, and pops (there is nothing left to sit an exam against).
const _pairingExpiredMessage =
    'Pairing expired. Pair again from the deck list menu.';

/// Phases `GET /api/remote/exam` cycles through while doing async work: the
/// screen keeps polling for as long as the last DTO reports one of these
/// (or `thinking`); `answering` is client-driven (no poll needed) and
/// `results`/`remediated`/`error` are terminal for their stage.
const _pollingPhases = {'generating', 'grading', 'remediating'};

/// A full-screen AI exam sitting on the paired desktop: one immersive flow,
/// mirroring how review_screen.dart structures itself (a plain header, the
/// deck name truncating, a Close affordance). Phase-driven off
/// [RemoteExam.phase]; see `docs/API.md` section 4.10 / `RemoteExamDto`.
class ExamScreen extends StatefulWidget {
  const ExamScreen({
    super.key,
    required this.deckName,
    required this.client,
    required this.support,
    required this.buildClient,
    required this.applyPassed,
    required this.applyRemediation,
    required this.nowMs,
    this.applyFailed,
    this.pollInterval = const Duration(milliseconds: 400),
  });

  /// The name the paired server's own catalog resolves this deck by: a bare
  /// top-level file name, or `<workspace>/<file>` for a workspace member.
  /// Never a device-absolute path; see review_screen.dart's `_deckName()`.
  final String deckName;

  /// The paired desktop's AI backend, over `/api/remote/*`.
  final ServerClient client;

  /// The support dir the "Re-pair" action reopens the pairing sheet
  /// against on a 401 (see `_expirePairingAndPop`); the caller (review or
  /// walk screen) resolves this once and hands it down.
  final Directory support;

  /// Builds the pairing sheet's own probe client, the same way the caller
  /// resolves its own.
  final ServerClient Function(ServerConfig) buildClient;

  /// Applies a PASSED sitting to the phone's own store (a closure over the
  /// bridge session's `applyExamPassed`). Called at most once per sitting.
  final void Function(BigInt nowMs) applyPassed;

  /// Turns a failed sitting's remediation deck-text into phone-store
  /// virtual cards (a closure over `applyRemediation`), returning how many
  /// were created or revived.
  final int Function(String cardsText, BigInt nowMs) applyRemediation;

  /// Records a FAILED trace exam so a re-sit waits out the cooldown; the
  /// phone owns this write (a closure over the walk session's
  /// `applyExamFailed`). Null for fact-deck exams, which never call it: a
  /// fact-deck fail remediates instead of persisting a cooldown-triggering
  /// failure.
  final void Function(BigInt nowMs)? applyFailed;

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
  bool _submitting = false;
  bool _remediating = false;

  /// Guards against a passed sitting applying its mastery more than once:
  /// a stray extra poll landing on the same terminal `results` DTO (e.g.
  /// two periodic ticks resolving out of order) must not double-apply.
  bool _passedApplied = false;

  /// Same guard, for turning `remediated` cards into store entries.
  bool _remediationApplied = false;

  /// Same guard, for persisting a failed trace exam (starts its cooldown).
  bool _failedApplied = false;

  /// How many consecutive null first-poll replies to tolerate before
  /// falling back to the refusal path (Polish 8b): a slow sitting start
  /// must not strand the screen, but a genuinely dead server must not spin
  /// forever either.
  static const _firstPollRetryLimit = 4;
  int _firstPollAttempts = 0;

  @override
  void initState() {
    super.initState();
    _start();
  }

  @override
  void dispose() {
    _pollTimer?.cancel();
    // Fire and forget: the sitting slot is dropped either way, and a reply
    // arriving after dispose (including a thrown PairingExpired) must not
    // surface as an unhandled error.
    widget.client.examClose().catchError((_) {});
    for (final c in _answerControllers) {
      c.dispose();
    }
    super.dispose();
  }

  // ── the phase walk ───────────────────────────────────────────────────

  Future<void> _start() async {
    bool ok;
    try {
      ok = await widget.client.examStart(widget.deckName);
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
    if (dto.phase == 'results' && dto.passed == true && !_passedApplied) {
      _passedApplied = true;
      widget.applyPassed(widget.nowMs());
    }
    if (dto.phase == 'results' &&
        dto.passed == false &&
        dto.isTrace &&
        widget.applyFailed != null &&
        !_failedApplied) {
      _failedApplied = true;
      widget.applyFailed!(widget.nowMs());
    }
    if (dto.phase == 'remediated' && !_remediationApplied) {
      _remediationApplied = true;
      // `cards` is non-null per the `remediated`-phase contract; an empty
      // fallback only guards the nullable type, never a real reply.
      final count = widget.applyRemediation(dto.cards ?? '', widget.nowMs());
      _snack('$count new cards to drill.');
    }
    setState(() {
      _exam = dto;
      if (dto.phase == 'answering' && _answerControllers.isEmpty) {
        _answerControllers.addAll(
          List.generate(dto.questions.length, (_) => TextEditingController()),
        );
      }
    });
    _rescheduleTimer(dto);
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
    return Scaffold(
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
    );
  }

  Widget _body(BuildContext context) {
    final exam = _exam;
    if (exam?.error != null) return _errorState(context);
    final phase = exam?.phase;
    if (phase == null || _pollingPhases.contains(phase)) {
      return _working(context, exam?.elapsed);
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
        return _working(context, exam?.elapsed);
    }
  }

  Widget _working(BuildContext context, int? elapsed) {
    final tokens = Theme.of(context).alix;
    final suffix = elapsed != null ? ' ${elapsed}s' : '';
    return Center(
      child: Text(
        'The server is working…$suffix',
        // Matches the web's `.exam-wait` idiom (dim + italic).
        style: TextStyle(color: tokens.dim, fontStyle: FontStyle.italic),
      ),
    );
  }

  Widget _errorState(BuildContext context) {
    final tokens = Theme.of(context).alix;
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          // Matches the web's `.exam-error` color token (red, not neutral).
          Text('The exam backend failed.', style: TextStyle(color: tokens.again)),
          const SizedBox(height: 16),
          OutlinedButton(
            onPressed: () => Navigator.of(context).maybePop(),
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
                  : () => setState(() => _currentQuestion++),
              child: Text(last ? 'Submit' : 'Next'),
            ),
          ],
        ),
      ],
    );
  }

  Widget _results(BuildContext context, RemoteExam exam) {
    final tokens = Theme.of(context).alix;
    final passed = exam.passed ?? false;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          passed ? 'Passed.' : 'Not yet.',
          // Matches the web's `.exam-pass`/`.exam-fail` color tokens.
          style: TextStyle(
              fontSize: 22,
              fontWeight: FontWeight.w600,
              color: passed ? tokens.good : tokens.again),
        ),
        const SizedBox(height: 16),
        Expanded(
          child: ListView(
            children: [
              for (final g in exam.grades) _gradeRow(g, tokens),
              if (!passed && exam.gaps.isNotEmpty) ...[
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
        if (!passed && exam.canRemediate) ...[
          const SizedBox(height: 12),
          FilledButton(
            onPressed: _remediating ? null : _remediate,
            child: const Text('Turn the gaps into cards'),
          ),
        ],
        if (!passed && exam.isTrace) ...[
          const SizedBox(height: 12),
          Text('Walk the trace again before re-sitting.', style: TextStyle(color: tokens.dim)),
        ],
      ],
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
          Text(g.verdict, style: TextStyle(fontWeight: FontWeight.w600, color: color)),
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
            onPressed: () => Navigator.of(context).maybePop(),
            child: const Text('Close'),
          ),
        ],
      ),
    );
  }
}
