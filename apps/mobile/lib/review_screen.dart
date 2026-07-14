import 'dart:io';

import 'package:flutter/material.dart';

import 'package:alix_mobile/src/rust/api/review.dart';

/// The review screen: renders the core's ReviewState and feeds the learner's
/// actions back. All review logic lives in Rust; this widget switches on
/// `acquire` and `mode` and forwards taps.
class ReviewScreen extends StatefulWidget {
  const ReviewScreen({
    super.key,
    required this.deckPath,
    required this.rootDir,
    required this.depth,
    this.device,
  });

  final String deckPath;
  final String rootDir;
  final Depth depth;

  /// This install's label for the store's last-writer marker; also enables
  /// the "another device wrote this recently" banner.
  final String? device;

  @override
  State<ReviewScreen> createState() => _ReviewScreenState();
}

class _ReviewScreenState extends State<ReviewScreen> {
  late ReviewSession _session;
  late ReviewState _state;
  bool _revealed = false;
  int _revealedLines = 0;
  ChoiceFeedback? _choice;
  CheckFeedback? _check;
  final List<TextEditingController> _typed = [];
  /// Explain: which keypoint rows are ticked, and the optional client-only
  /// attempt (never sent, never graded; a slot a draw canvas can fill later).
  final Set<int> _ticked = {};
  bool _attemptOpen = false;
  final TextEditingController _attempt = TextEditingController();

  @override
  void initState() {
    super.initState();
    _open();
  }

  @override
  void dispose() {
    for (final c in _typed) {
      c.dispose();
    }
    _attempt.dispose();
    super.dispose();
  }

  /// Another device's recent write of this store, shown once per session
  /// open until dismissed.
  ForeignWriter? _foreign;

  /// (Re)opens the session; also the restart action on the done screen.
  void _open() {
    _session = ReviewSession.open(
      deckPath: widget.deckPath,
      rootDir: widget.rootDir,
      depth: widget.depth,
      device: widget.device,
    );
    _foreign = widget.device == null ? null : _session.foreignWriter();
    _resetCard(_session.state());
  }

  /// Installs the next position and clears all per-card interaction state.
  void _resetCard(ReviewState next) {
    _state = next;
    _revealed = false;
    _revealedLines = 1;
    _choice = null;
    _check = null;
    _ticked.clear();
    _attemptOpen = false;
    _attempt.clear();
    final lines = next.mode == Mode.typeLine ? (next.card?.back.length ?? 1) : 1;
    while (_typed.length < lines) {
      _typed.add(TextEditingController());
    }
    for (final c in _typed) {
      c.clear();
    }
  }

  void _apply(ReviewState next) => setState(() => _resetCard(next));

  void _grade(Grade grade) => _apply(_session.grade(grade: grade));

  @override
  Widget build(BuildContext context) {
    final card = _state.card;
    return Scaffold(
      appBar: AppBar(
        title: const Text('alix'),
        actions: [
          if (!_state.finished)
            Padding(
              padding: const EdgeInsets.only(right: 16),
              child: Center(child: Text('${_state.remaining} left')),
            ),
        ],
      ),
      body: SafeArea(
        child: Column(
          children: [
            if (_foreign case final foreign?) _foreignBanner(context, foreign),
            Expanded(
              child: Padding(
                padding: const EdgeInsets.all(24),
                child: card == null ? _done(context) : _card(context, card),
              ),
            ),
          ],
        ),
      ),
    );
  }

  /// The roaming-discipline warning: this store was just written elsewhere,
  /// so reviewing here would fork it.
  Widget _foreignBanner(BuildContext context, ForeignWriter foreign) {
    final theme = Theme.of(context);
    final minutes = (foreign.ageMs.toInt() / 60000).round();
    final age = minutes < 1 ? 'moments' : '$minutes min';
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
              "Last written by '${foreign.device}' $age ago. "
              'Review on one device at a time.',
              style: theme.textTheme.bodySmall
                  ?.copyWith(color: theme.colorScheme.onErrorContainer),
            ),
          ),
          IconButton(
            icon: const Icon(Icons.close, size: 18),
            onPressed: () => setState(() => _foreign = null),
          ),
        ],
      ),
    );
  }

  Widget _done(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text('Done for now', style: Theme.of(context).textTheme.headlineSmall),
          const SizedBox(height: 8),
          const Text('Cards come back once they are due again.'),
          const SizedBox(height: 24),
          OutlinedButton(
            onPressed: () => setState(_open),
            child: const Text('Check again'),
          ),
        ],
      ),
    );
  }

  Widget _card(BuildContext context, CardView card) {
    return Column(
      children: [
        Expanded(
          child: SingleChildScrollView(
            // The scroll view hands loose width constraints; force full
            // width so the face column actually centers.
            child: SizedBox(width: double.infinity, child: _face(context, card)),
          ),
        ),
        const SizedBox(height: 12),
        Center(child: _actions(context, card)),
      ],
    );
  }

  /// The card face: front, cloze context, then whatever the current phase of
  /// the current mode has uncovered.
  Widget _face(BuildContext context, CardView card) {
    final theme = Theme.of(context);
    final answerStyle =
        theme.textTheme.titleLarge?.copyWith(color: theme.colorScheme.primary);
    final showBack = _state.acquire && _state.choices == null ||
        _revealed ||
        _check != null;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        const SizedBox(height: 24),
        Text(
          card.front,
          textAlign: TextAlign.center,
          style: theme.textTheme.headlineSmall,
        ),
        if (card.image != null) ...[
          const SizedBox(height: 12),
          Image.file(File(card.image!), height: 180),
        ],
        for (final line in card.context) ...[
          const SizedBox(height: 8),
          Text(line, textAlign: TextAlign.center, style: theme.textTheme.titleMedium),
        ],
        const SizedBox(height: 24),
        if (_state.choices != null)
          _options(context)
        else if (_state.mode == Mode.lineByLine && !_state.acquire) ...[
          for (final line in card.back.take(_revealedLines))
            Text(line, textAlign: TextAlign.center, style: answerStyle),
        ] else if (_isTyping && !_state.acquire)
          _typing(context, card)
        else if (_isExplain) ...[
          if (!_revealed)
            _attemptSlot(context)
          else ...[
            if (_attempt.text.trim().isNotEmpty) ...[
              Text(
                _attempt.text.trim(),
                textAlign: TextAlign.center,
                style: theme.textTheme.bodyMedium
                    ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
              ),
              const SizedBox(height: 16),
            ],
            for (final line in card.back)
              Text(line, textAlign: TextAlign.center, style: answerStyle),
            const SizedBox(height: 16),
            _checklist(context),
          ],
        ] else if (showBack) ...[
          for (final line in card.back)
            Text(line, textAlign: TextAlign.center, style: answerStyle),
        ],
        if (showBack || _lineDone(card) || _choice != null) ...[
          if (card.imageBack != null) ...[
            const SizedBox(height: 12),
            Image.file(File(card.imageBack!), height: 180),
          ],
          for (final note in card.note) ...[
            const SizedBox(height: 8),
            switch (note) {
              NoteUnit_Sentence(:final text) => Text(
                  text,
                  textAlign: TextAlign.center,
                  style: theme.textTheme.bodyMedium
                      ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
                ),
              NoteUnit_Code(:final lines) => Text(
                  lines.join('\n'),
                  textAlign: TextAlign.left,
                  style: theme.textTheme.bodyMedium?.copyWith(
                    color: theme.colorScheme.onSurfaceVariant,
                    fontFamily: 'monospace',
                  ),
                ),
            },
          ],
        ],
      ],
    );
  }

  bool get _isTyping =>
      _state.mode == Mode.typing || _state.mode == Mode.typeLine;

  /// Explain past acquire always carries a rubric (core fills the fallback).
  bool get _isExplain =>
      _state.mode == Mode.explain &&
      !_state.acquire &&
      _state.keypoints != null;

  /// The optional client-only attempt: collapsed on every device; a phone
  /// ignores it, a tablet gets its keyboard. Never sent, never graded.
  Widget _attemptSlot(BuildContext context) {
    if (!_attemptOpen) {
      return TextButton(
        onPressed: () => setState(() => _attemptOpen = true),
        child: const Text('type it first'),
      );
    }
    return TextField(
      controller: _attempt,
      minLines: 2,
      maxLines: 5,
      decoration: const InputDecoration(
        border: OutlineInputBorder(),
        labelText: 'your explanation (stays on this device)',
      ),
    );
  }

  /// The tick-each-keypoint rubric plus a live verdict hint; Continue in the
  /// action row commits the tally as the grade.
  Widget _checklist(BuildContext context) {
    final theme = Theme.of(context);
    final points = _state.keypoints ?? const [];
    final hint = theme.textTheme.bodyMedium
        ?.copyWith(color: theme.colorScheme.onSurfaceVariant);
    return Column(
      children: [
        Text('did you cover these?', style: hint),
        const SizedBox(height: 4),
        for (final (i, point) in points.indexed)
          CheckboxListTile(
            dense: true,
            controlAffinity: ListTileControlAffinity.leading,
            value: _ticked.contains(i),
            onChanged: (_) => setState(
              () => _ticked.contains(i) ? _ticked.remove(i) : _ticked.add(i),
            ),
            title: Text(point),
          ),
        const SizedBox(height: 4),
        Text('${_ticked.length}/${points.length} → ${_verdict()}', style: hint),
      ],
    );
  }

  String _verdict() {
    final grade = keypointGrade(
      covered: _ticked.length,
      total: (_state.keypoints ?? const []).length,
    );
    return switch (grade) {
      Grade.fail => 'fail',
      Grade.partial => 'partial',
      Grade.pass => 'pass',
    };
  }

  bool _lineDone(CardView card) =>
      _state.mode == Mode.lineByLine && _revealedLines >= card.back.length;

  /// Multiple-choice options, tinted once a pick was made.
  Widget _options(BuildContext context) {
    final options = _state.choices ?? const [];
    final scheme = Theme.of(context).colorScheme;
    return Column(
      children: [
        for (final (i, option) in options.indexed)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 4),
            child: SizedBox(
              width: double.infinity,
              child: OutlinedButton(
                style: _choice == null
                    ? null
                    : OutlinedButton.styleFrom(
                        backgroundColor: BigInt.from(i) == _choice!.correct
                            ? scheme.primaryContainer
                            : (BigInt.from(i) == _choice!.chosen
                                ? scheme.errorContainer
                                : null),
                      ),
                onPressed: _choice == null
                    ? () => setState(() => _choice = _session.choose(chosen: i))
                    : null,
                child: Text(option, maxLines: 2, overflow: TextOverflow.ellipsis),
              ),
            ),
          ),
      ],
    );
  }

  /// Typed-answer entry plus, after checking, the per-line evidence.
  Widget _typing(BuildContext context, CardView card) {
    final scheme = Theme.of(context).colorScheme;
    final fields = _state.mode == Mode.typeLine ? card.back.length : 1;
    return Column(
      children: [
        for (var i = 0; i < fields; i++)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 4),
            child: TextField(
              controller: _typed[i],
              enabled: _check == null,
              decoration: InputDecoration(
                border: const OutlineInputBorder(),
                labelText: fields > 1 ? 'line ${i + 1}' : 'your answer',
              ),
            ),
          ),
        if (_check != null) ...[
          const SizedBox(height: 8),
          for (final result in _check!.results)
            Text(
              result.passed ? result.expected : '${result.input} -> ${result.expected}',
              style: TextStyle(
                color: result.passed ? scheme.primary : scheme.error,
              ),
            ),
        ],
      ],
    );
  }

  /// The bottom action row for the current phase.
  Widget _actions(BuildContext context, CardView card) {
    // First encounter: acknowledge, never grade.
    if (_state.acquire) {
      if (_state.choices != null && _choice == null) {
        return const Text('pick what you think it is');
      }
      return FilledButton(
        onPressed: () => _apply(_session.acquire()),
        child: const Text('Seen'),
      );
    }
    // A graded pick: web-parity mapping (right = Pass or own up to a guess;
    // wrong = continue as a Fail).
    if (_state.mode == Mode.choice) {
      final choice = _choice;
      if (choice == null) return const SizedBox.shrink();
      return choice.passed
          ? _row([
              ('I guessed', () => _grade(Grade.fail)),
              ('Next', () => _grade(Grade.pass)),
            ])
          : _row([('Continue', () => _grade(Grade.fail))]);
    }
    if (_state.mode == Mode.lineByLine) {
      if (!_lineDone(card)) {
        return FilledButton(
          onPressed: () => setState(() => _revealedLines++),
          child: const Text('Next line'),
        );
      }
      return _gradeRow();
    }
    if (_isTyping) {
      if (_check == null) {
        return FilledButton(
          onPressed: () => setState(() {
            final fields = _state.mode == Mode.typeLine ? card.back.length : 1;
            _check = _session.check(
              lines: [for (var i = 0; i < fields; i++) _typed[i].text],
            );
          }),
          child: const Text('Check'),
        );
      }
      return _gradeRow();
    }
    // Explain: the ticks ARE the grade (the same keypoint_grade rule the web
    // submits {covered, total} to), so Continue replaces the grade row.
    if (_isExplain && _revealed) {
      return FilledButton(
        onPressed: () => _grade(keypointGrade(
          covered: _ticked.length,
          total: _state.keypoints!.length,
        )),
        child: const Text('Continue'),
      );
    }
    // Flip (and Explain before its reveal).
    if (!_revealed) {
      return FilledButton(
        onPressed: () => setState(() => _revealed = true),
        child: const Text('Reveal'),
      );
    }
    return _gradeRow();
  }

  Widget _gradeRow() => _row([
        ('Fail', () => _grade(Grade.fail)),
        ('Partial', () => _grade(Grade.partial)),
        ('Pass', () => _grade(Grade.pass)),
      ]);

  Widget _row(List<(String, VoidCallback)> actions) {
    return Row(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        for (final (label, action) in actions)
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 6),
            child: FilledButton.tonal(onPressed: action, child: Text(label)),
          ),
      ],
    );
  }
}
