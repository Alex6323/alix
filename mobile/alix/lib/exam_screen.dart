import 'dart:async';

import 'package:flutter/foundation.dart' show listEquals;
import 'package:flutter/material.dart';

import 'package:alix_mobile/bridge/bridge_error.dart';
import 'package:alix_mobile/bridge/exam_bridge.dart' show AnswerTooLong, HeldExamAccess, HeldExamView;
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/unit_widgets.dart';
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

const pairingExpiredMessage =
    'Pairing expired. Pair again from Settings → Connected devices.';

const _needsDesktop = 'grading needs the paired desktop';
const _busy = 'the desktop is busy grading; try again shortly';
const _unreadable = 'the desktop could not read this exam; update alix on both devices';
const _staleExam =
    'The deck has changed. This exam is stale and cannot be recorded. Discard it and generate a new one.';

enum _Desk {
  grade(running: 'grading', finished: 'results'),
  remediate(running: 'remediating', finished: 'remediated');

  const _Desk({required this.running, required this.finished});

  final String running;
  final String finished;
}

enum _Stage { opening, unreadable, stale, answers, grading, unapplied, result }

class ExamScreen extends StatefulWidget {
  const ExamScreen({
    super.key,
    required this.deckName,
    required this.held,
    required this.client,
    required this.onLeaveResult,
    required this.onDiscard,
    this.pollInterval = const Duration(milliseconds: 400),
  });

  final String deckName;
  final HeldExamAccess held;
  final ServerClient? client;
  final VoidCallback onLeaveResult;
  final Future<bool> Function() onDiscard;
  final Duration pollInterval;

  @override
  State<ExamScreen> createState() => _ExamScreenState();
}

class _ExamScreenState extends State<ExamScreen> with WidgetsBindingObserver {
  _Stage _stage = _Stage.opening;
  HeldExamView? _view;
  String? _line;
  String? _aside;
  bool _again = false;
  bool _stale = false;
  int _closesPending = 0;
  bool _remediating = false;
  bool _discarding = false;
  bool _leaving = false;
  bool _foreground = true;
  int? _elapsed;
  Timer? _pollTimer;
  Future<void> _closing = Future<void>.value();

  int _currentQuestion = 0;
  final List<TextEditingController> _answerControllers = [];
  final List<bool> _answerLocked = [];
  List<String> _savedAnswers = const [];
  List<bool> _savedLocked = const [];

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _open();
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _pollTimer?.cancel();
    final client = widget.client;
    if (client != null) unawaited(_closing.whenComplete(client.close));
    for (final c in _answerControllers) {
      c.dispose();
    }
    super.dispose();
  }

  void _open() {
    final HeldExamView? view;
    try {
      view = widget.held.view();
    } on Object catch (error) {
      _stage = _Stage.unreadable;
      _line = bridgeErrorText(error);
      return;
    }
    if (view == null) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _pop());
      return;
    }
    _view = view;
    if (view.result == null) {
      _installAnswers(view);
      _stage = _Stage.answers;
      _materialChanged();
      if (_stale) _stage = _Stage.stale;
    } else if (view.outcome == null) {
      _apply();
    } else {
      _stage = _Stage.result;
    }
  }

  void _installAnswers(HeldExamView view) {
    _answerControllers.addAll([
      for (final answer in view.answers) TextEditingController(text: answer),
    ]);
    _answerLocked.addAll(view.locked);
    _savedAnswers = List.of(view.answers);
    _savedLocked = List.of(view.locked);
    final firstOpen = _answerLocked.indexWhere((locked) => !locked);
    _currentQuestion = firstOpen >= 0
        ? firstOpen
        : (view.questions.isEmpty ? 0 : view.questions.length - 1);
  }

  bool _materialChanged() {
    try {
      if (!widget.held.materialChanged()) return false;
      _stale = true;
      _say(null);
    } on Object catch (error) {
      _say(bridgeErrorText(error));
    }
    return true;
  }

  void _say(String? line, {bool again = false}) {
    _line = line;
    _aside = null;
    _again = again;
  }

  String? get _shown => _aside ?? _line;

  void _apply() {
    try {
      _view = widget.held.apply();
      _stage = _Stage.result;
      _say(null);
    } on Object catch (error) {
      _stage = _Stage.unapplied;
      _say(bridgeErrorText(error));
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (_foreground && state != AppLifecycleState.resumed) _saveQuietly();
    _foreground = state == AppLifecycleState.resumed;
  }

  void _save() {
    final answers = [for (final controller in _answerControllers) controller.text];
    final locked = List.of(_answerLocked);
    if (listEquals(answers, _savedAnswers) && listEquals(locked, _savedLocked)) return;
    widget.held.saveAnswers(answers, locked);
    _savedAnswers = answers;
    _savedLocked = locked;
  }

  void _saveQuietly() {
    if (_stage != _Stage.answers || _answerControllers.isEmpty) return;
    try {
      _save();
    } on Object catch (_) {
      return;
    }
  }

  bool _lockAndSave() {
    final index = _currentQuestion;
    final locking = !_answerLocked[index] && _answerControllers[index].text.trim().isNotEmpty;
    if (locking) _answerLocked[index] = true;
    try {
      _save();
      return true;
    } on Object catch (error) {
      if (locking) _answerLocked[index] = false;
      setState(() => _aside = error is AnswerTooLong ? '$error' : bridgeErrorText(error));
      return false;
    }
  }

  void _nextQuestion() {
    if (!_lockAndSave()) return;
    setState(() {
      _currentQuestion++;
      _aside = null;
      if (!_again) _line = null;
    });
  }

  void _previousQuestion() {
    _saveQuietly();
    setState(() {
      _currentQuestion--;
      _aside = null;
    });
  }

  bool get _departing => _leaving || _discarding;

  Future<void> _submit() async {
    if (_stage != _Stage.answers || _departing || !_lockAndSave()) return;
    if (_materialChanged()) {
      setState(() {});
      return;
    }
    final String request;
    try {
      request = widget.held.gradeRequest();
    } on AnswerTooLong catch (error) {
      setState(() => _aside = '$error');
      return;
    } on Object catch (error) {
      setState(() => _say(bridgeErrorText(error), again: true));
      return;
    }
    final client = widget.client;
    if (client == null) {
      setState(() => _say(_needsDesktop, again: true));
      return;
    }
    setState(() {
      _stage = _Stage.grading;
      _elapsed = null;
      _say(null);
    });
    await _ask(_Desk.grade, client, () => client.gradingSubmit(request));
  }

  Future<void> _remediate() async {
    if (_remediating || _closesPending > 0 || _departing) return;
    final String? request;
    try {
      request = widget.held.remediationRequest();
    } on Object catch (error) {
      setState(() => _fail(_Desk.remediate, bridgeErrorText(error)));
      return;
    }
    if (request == null) return;
    final client = widget.client;
    if (client == null) {
      setState(() => _fail(_Desk.remediate, _needsDesktop));
      return;
    }
    setState(() {
      _remediating = true;
      _elapsed = null;
      _say(null);
    });
    final body = request;
    await _ask(_Desk.remediate, client, () => client.gradingRemediate(body));
  }

  Future<void> _ask(_Desk desk, ServerClient client, Future<GradingCall> Function() call) async {
    final GradingCall reply;
    try {
      reply = await call();
    } on PairingExpired {
      if (mounted) setState(() => _fail(desk, pairingExpiredMessage));
      return;
    }
    if (!mounted) return;
    final sitting = _view!.sitting;
    switch (reply) {
      case GradingNoAnswer():
        setState(() => _fail(desk, _needsDesktop));
      case GradingRefused(:final status):
        setState(() => _fail(desk, switch (status) {
          409 => _busy,
          400 => _unreadable,
          _ => 'the desktop refused this request ($status)',
        }));
      case GradingAnswered(:final state) when state.sitting != sitting:
        setState(() => _fail(desk, null));
      case GradingAnswered(:final state) when state.error != null:
        setState(() => _fail(desk, state.error));
      case GradingAnswered(:final state) when state.phase == desk.running:
        setState(() => _elapsed = state.elapsed);
        _pollTimer = Timer(widget.pollInterval, () => _ask(desk, client, client.gradingGet));
      case GradingAnswered(:final body, :final state) when state.phase == desk.finished:
        if (desk == _Desk.grade) {
          setState(() => _storeGrade(client, body));
        } else {
          _storeRemediation(client, body);
        }
      case GradingAnswered():
        setState(() => _fail(desk, null));
    }
  }

  void _fail(_Desk desk, String? line) {
    if (desk == _Desk.grade) {
      _stage = _Stage.answers;
    } else {
      _remediating = false;
    }
    _say(line, again: true);
  }

  void _storeGrade(ServerClient client, String body) {
    try {
      widget.held.storeResult(body);
    } on Object catch (error) {
      _fail(_Desk.grade, bridgeErrorText(error));
      return;
    }
    _closeDesk(client);
    _apply();
  }

  void _storeRemediation(ServerClient client, String body) {
    final int count;
    try {
      count = widget.held.storeRemediation(body);
    } on Object catch (error) {
      setState(() => _fail(_Desk.remediate, bridgeErrorText(error)));
      return;
    }
    _closeDesk(client);
    final HeldExamView? view;
    try {
      view = widget.held.view();
    } on Object catch (error) {
      setState(() {
        _remediating = false;
        _stage = _Stage.unreadable;
        _say(bridgeErrorText(error));
      });
      return;
    }
    if (view == null) {
      _pop();
      return;
    }
    setState(() {
      _view = view;
      _remediating = false;
      _say(null);
    });
    if (count > 0) _snack('$count new cards to drill.');
  }

  void _closeDesk(ServerClient client) {
    _closesPending++;
    final close = client
        .gradingClose(_view!.sitting)
        .then<void>((_) {}, onError: (Object _) {})
        .whenComplete(() {
          _closesPending--;
          if (mounted) setState(() {});
        });
    _closing = Future.wait([_closing, close]);
  }

  void _leave() {
    if (_leaving) return;
    _leaving = true;
    _pollTimer?.cancel();
    if (_stage == _Stage.answers) _saveQuietly();
    try {
      if (_stage == _Stage.result && _view?.outcome != null && !_remediating) widget.onLeaveResult();
    } on Object catch (_) {
      return;
    } finally {
      _pop();
    }
  }

  Future<void> _discard() async {
    if (_departing) return;
    setState(() {
      _discarding = true;
      _aside = null;
    });
    final bool discarded;
    try {
      discarded = await widget.onDiscard();
    } on Object catch (error) {
      if (mounted) {
        setState(() {
          _discarding = false;
          _aside = bridgeErrorText(error);
        });
      }
      return;
    }
    if (!mounted) return;
    if (!discarded) {
      setState(() => _discarding = false);
      return;
    }
    _leaving = true;
    _pollTimer?.cancel();
    _pop();
  }

  void _pop() {
    if (!mounted) return;
    final navigator = Navigator.of(context);
    if (navigator.canPop()) navigator.pop();
  }

  void _snack(String text) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(text)));
  }

  @override
  Widget build(BuildContext context) {
    return PopScope(
      canPop: false,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) _leave();
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

  Widget _body(BuildContext context) => switch (_stage) {
    _Stage.opening => const SizedBox.shrink(),
    _Stage.unreadable => _message(context, null),
    _Stage.stale => _message(context, OutlinedButton(onPressed: _discard, child: const Text('Discard exam'))),
    _Stage.answers => _answering(context, _view!),
    _Stage.grading => _working(context),
    _Stage.unapplied => _message(context, OutlinedButton(onPressed: () => setState(_apply), child: const Text('try again'))),
    _Stage.result => _results(context, _view!),
  };

  Widget _message(BuildContext context, Widget? action) {
    final tokens = Theme.of(context).alix;
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(
            _noticeLine ?? '',
            textAlign: TextAlign.center,
            maxLines: _noticeMaxLines,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(color: tokens.again),
          ),
          if (action != null) ...[const SizedBox(height: 16), action],
        ],
      ),
    );
  }

  Widget _working(BuildContext context) {
    final tokens = Theme.of(context).alix;
    final suffix = _elapsed != null ? ' ${_elapsed}s' : '';
    return Center(
      child: Text(
        'The server is working…$suffix',
        style: TextStyle(color: tokens.dim, fontStyle: FontStyle.italic),
      ),
    );
  }

  Widget _notice(BuildContext context) {
    final tokens = Theme.of(context).alix;
    return Text(
      _noticeLine ?? '',
      key: const ValueKey('exam-notice'),
      maxLines: _noticeMaxLines,
      overflow: TextOverflow.ellipsis,
      style: TextStyle(color: tokens.again, fontSize: 13),
    );
  }

  String? get _noticeLine => _shown ?? (_stale ? _staleExam : null);

  int get _noticeMaxLines => _noticeLine == _staleExam ? 8 : 3;

  bool get _hasNotice => _noticeLine != null;

  String get _submitLabel => _stale ? 'Discard exam' : (_again ? 'submit again' : 'Submit');

  Widget _answering(BuildContext context, HeldExamView exam) {
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
        Flexible(
          child: _QuestionScroll(
            key: ValueKey('exam-question-scroll-$index'),
            child: _question(context, exam, index),
          ),
        ),
        const SizedBox(height: 12),
        Divider(
          key: const ValueKey('exam-question-hairline'),
          height: 1,
          thickness: 1,
          color: tokens.line,
        ),
        const SizedBox(height: 12),
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
        if (_hasNotice) ...[const SizedBox(height: 12), _notice(context)],
        const SizedBox(height: 12),
        LayoutBuilder(
          builder: (context, constraints) => Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              OutlinedButton(
                style: OutlinedButton.styleFrom(padding: const EdgeInsets.symmetric(horizontal: 16)),
                onPressed: index > 0 ? _previousQuestion : null,
                child: const Text('Back'),
              ),
              Flexible(
                child: TextButton(
                  style: TextButton.styleFrom(minimumSize: const Size(0, 40)),
                  onPressed: () => Navigator.of(context).maybePop(),
                  child: const Text(
                    'Continue later',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ),
              ConstrainedBox(
                constraints: BoxConstraints(maxWidth: constraints.maxWidth * 0.55),
                child: FilledButton(
                  style: FilledButton.styleFrom(padding: const EdgeInsets.symmetric(horizontal: 16)),
                  onPressed: last ? (_stale ? _discard : _submit) : _nextQuestion,
                  child: Text(last ? _submitLabel : 'Next', maxLines: 1, overflow: TextOverflow.ellipsis),
                ),
              ),
            ],
          ),
        ),
      ],
    );
  }

  Widget _question(BuildContext context, HeldExamView exam, int index) {
    final style = Theme.of(context).textTheme.titleMedium ?? const TextStyle();
    final units = index < exam.questionsUnits.length ? exam.questionsUnits[index] : const <ReviewContentUnitModel>[];
    return _examText(context, units, exam.questions[index], style);
  }

  Widget _examText(BuildContext context, List<ReviewContentUnitModel> units, String text, TextStyle style) {
    if (units.isEmpty) return Text(text, style: style);
    final tokens = Theme.of(context).alix;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final unit in units)
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: switch (unit) {
              ReviewCodeModel(:final lines) => _numberedCode(lines, style.color ?? tokens.text, tokens),
              _ => unitWidget(unit, tokens, style, TextAlign.start),
            },
          ),
      ],
    );
  }

  Widget _numberedCode(List<String> lines, Color foreground, AlixTokens tokens) {
    final style = TextStyle(fontFamily: monoFontFamily, fontSize: 13, height: 1.45, color: foreground);
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
      decoration: BoxDecoration(
        color: Colors.black.withValues(alpha: 0.32),
        borderRadius: BorderRadius.circular(8),
      ),
      child: Table(
        columnWidths: const {0: IntrinsicColumnWidth(), 1: FlexColumnWidth()},
        children: [
          for (final (index, line) in lines.indexed)
            TableRow(
              children: [
                Padding(
                  padding: const EdgeInsets.only(right: 12),
                  child: Text(
                    '${index + 1}',
                    textAlign: TextAlign.right,
                    softWrap: false,
                    style: style.copyWith(color: tokens.faint),
                  ),
                ),
                Text(line, style: style),
              ],
            ),
        ],
      ),
    );
  }


  Widget _results(BuildContext context, HeldExamView view) {
    final tokens = Theme.of(context).alix;
    final result = view.result!;
    final verdicts = result.verdicts;
    final outcome = view.outcome;
    final standing = outcome?.standing;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          '${_points(verdicts.points)} / ${verdicts.total}',
          style: TextStyle(
            fontSize: 22,
            fontWeight: FontWeight.w600,
            color: tokens.text,
          ),
        ),
        if (outcome != null && !outcome.recorded)
          Text('not recorded', style: TextStyle(color: tokens.dim, fontSize: 13))
        else if (standing != null)
          Text(
            standing.text,
            style: TextStyle(
              color: tokens.good,
              fontSize: 13,
              decoration: standing.examChanged ? TextDecoration.underline : null,
              decorationStyle: TextDecorationStyle.dashed,
            ),
          ),
        const SizedBox(height: 16),
        Expanded(
          child: ListView(
            children: [
              for (final topic in result.topics) _topicRow(topic, tokens),
              if (result.topics.isNotEmpty) const SizedBox(height: 8),
              if (view.unsourcedTopics.isNotEmpty)
                Text(
                  'Not examined: ${view.unsourcedTopics.join(', ')}',
                  style: TextStyle(color: tokens.dim, fontSize: 13),
                ),
              for (final g in result.grades) _gradeRow(context, g, tokens),
            ],
          ),
        ),
        if (_hasNotice) ...[const SizedBox(height: 12), _notice(context)],
        if (_remediating) ...[const SizedBox(height: 12), _working(context)],
        if (view.canRemediate) ...[
          const SizedBox(height: 12),
          FilledButton(
            onPressed: _remediating || _closesPending > 0 ? null : _remediate,
            child: Text(_again ? 'remediate again' : 'Turn the gaps into cards'),
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

  List<Widget> _pointList(
    BuildContext context,
    String label,
    List<String> points,
    List<List<ReviewContentUnitModel>> units,
    AlixTokens tokens,
  ) {
    if (points.isEmpty) return const [];
    final style = TextStyle(color: tokens.dim);
    return [
      const SizedBox(height: 6),
      Text(label, style: TextStyle(color: tokens.faint, fontSize: 12)),
      for (var i = 0; i < points.length; i++)
        Padding(
          padding: const EdgeInsets.only(left: 8, top: 2),
          child: _examText(context, i < units.length ? units[i] : const [], points[i], style),
        ),
    ];
  }

  Widget _gradeRow(BuildContext context, RemoteExamGrade g, AlixTokens tokens) {
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
          if (g.shape.isNotEmpty) Text(g.shape, style: TextStyle(color: tokens.faint, fontSize: 11)),
          _examText(context, g.questionUnits, g.question, TextStyle(color: tokens.text)),
          const SizedBox(height: 4),
          Text(g.verdict, style: TextStyle(fontWeight: FontWeight.w600, color: color)),
          _examText(context, g.feedbackUnits, g.feedback, TextStyle(color: tokens.dim)),
          if (g.verdict != 'PASS') ...[
            ..._pointList(context, 'A complete answer covers:', g.points, g.pointsUnits, tokens),
            ..._pointList(context, 'You missed:', g.missed, g.missedUnits, tokens),
          ],
        ],
      ),
    );
  }

}

class _QuestionScroll extends StatefulWidget {
  const _QuestionScroll({super.key, required this.child});

  final Widget child;

  @override
  State<_QuestionScroll> createState() => _QuestionScrollState();
}

class _QuestionScrollState extends State<_QuestionScroll> {
  final _controller = ScrollController();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scrollbar(
      controller: _controller,
      thumbVisibility: true,
      child: SingleChildScrollView(
        key: const ValueKey('exam-question'),
        controller: _controller,
        padding: const EdgeInsets.only(right: 12),
        child: widget.child,
      ),
    );
  }
}
