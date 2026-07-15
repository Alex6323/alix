import 'dart:io';

import 'package:flutter/material.dart';

import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/theme.dart';

const _mono = 'IBM Plex Mono';
const _sans = 'IBM Plex Sans';

/// The review screen: renders the core's ReviewState and feeds the learner's
/// actions back. All review logic lives in Rust; this widget switches on
/// `acquire` and `mode` and forwards taps. The surface mirrors the web
/// client (assets/web/review.html): a mono mode-tag, a bold question over a
/// faded divider, mode-specific answer bodies, a warm boxed note, and the
/// web's chip legend.
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

  /// Explain: which keypoint rows are ticked (covered).
  final Set<int> _ticked = {};

  /// Another device's recent write of this store, shown once per session
  /// open until dismissed.
  ForeignWriter? _foreign;

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
    super.dispose();
  }

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

  bool get _hasChoices =>
      _state.choices != null && _state.choices!.isNotEmpty;
  bool get _isTyping =>
      _state.mode == Mode.typing || _state.mode == Mode.typeLine;
  bool get _isExplain =>
      _state.mode == Mode.explain &&
      !_state.acquire &&
      _state.keypoints != null;
  bool _lineDone(CardView card) =>
      _state.mode == Mode.lineByLine && _revealedLines >= card.back.length;

  @override
  Widget build(BuildContext context) {
    final card = _state.card;
    return Scaffold(
      appBar: AppBar(
        title: const AlixWordmark(),
        actions: [
          if (!_state.finished)
            Padding(
              padding: const EdgeInsets.only(right: 16),
              child: Center(
                child: Text(
                  '${_state.remaining} left',
                  style: TextStyle(
                    fontFamily: _mono,
                    fontSize: 13,
                    color: Theme.of(context).alix.dim,
                  ),
                ),
              ),
            ),
        ],
      ),
      body: SafeArea(
        child: Column(
          children: [
            if (_foreign case final foreign?) _foreignBanner(context, foreign),
            Expanded(
              child: card == null
                  ? _done(context)
                  : Column(
                      children: [
                        Expanded(
                          child: SingleChildScrollView(
                            padding: const EdgeInsets.fromLTRB(20, 8, 20, 8),
                            child: SizedBox(
                              width: double.infinity,
                              child: _face(context, card),
                            ),
                          ),
                        ),
                        _legend(context, card),
                      ],
                    ),
            ),
          ],
        ),
      ),
    );
  }

  // ── card face ──────────────────────────────────────────────────────────

  /// The mode-tag label, matching the web's modeLabel().
  String _modeLabel() {
    if (_state.acquire) return 'new';
    if (_hasChoices) return 'choice';
    return switch (_state.mode) {
      Mode.typeLine => 'typing · line',
      Mode.typing => 'typing',
      Mode.explain => 'explain',
      Mode.lineByLine => 'line',
      Mode.choice => 'choice',
      Mode.flip => 'flip',
    };
  }

  Widget _face(BuildContext context, CardView card) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    final answered = _revealed || _check != null || _choice != null;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        const SizedBox(height: 8),
        _modeTag(_modeLabel(), tokens),
        const SizedBox(height: 12),
        Text(
          card.front,
          textAlign: TextAlign.center,
          style: TextStyle(
            fontFamily: _sans,
            fontWeight: FontWeight.w700,
            fontSize: 23,
            height: 1.25,
            color: theme.colorScheme.onSurface,
          ),
        ),
        if (card.image != null) ...[
          const SizedBox(height: 12),
          Image.file(File(card.image!), height: 180),
        ],
        for (final line in card.context) ...[
          const SizedBox(height: 8),
          Text(
            line,
            textAlign: TextAlign.center,
            style: theme.textTheme.titleMedium?.copyWith(color: tokens.text),
          ),
        ],
        const SizedBox(height: 14),
        _divider(tokens),
        const SizedBox(height: 22),
        _body(context, card, tokens),
        if (_state.acquire && !_hasChoices && !_revealed) ...[
          const SizedBox(height: 18),
          Text(
            'new card: try to recall it, then reveal.',
            textAlign: TextAlign.center,
            style: TextStyle(
              color: tokens.dim,
              fontSize: 13,
              fontStyle: FontStyle.italic,
            ),
          ),
        ],
        if (answered && card.note.isNotEmpty) _note(card, tokens),
      ],
    );
  }

  Widget _modeTag(String label, AlixTokens tokens) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 3),
      decoration: BoxDecoration(
        border: Border.all(color: tokens.line),
        borderRadius: BorderRadius.circular(6),
      ),
      child: Text(
        label.toUpperCase(),
        style: TextStyle(
          fontFamily: _mono,
          fontSize: 10.5,
          letterSpacing: 1.7,
          color: tokens.faint,
        ),
      ),
    );
  }

  /// A hairline that fades at both ends, 70% of the card width.
  Widget _divider(AlixTokens tokens) {
    return FractionallySizedBox(
      widthFactor: 0.7,
      child: Container(
        height: 1,
        decoration: BoxDecoration(
          gradient: LinearGradient(
            colors: [Colors.transparent, tokens.line, tokens.line, Colors.transparent],
            stops: const [0, 0.18, 0.82, 1],
          ),
        ),
      ),
    );
  }

  /// The mode-specific answer body.
  Widget _body(BuildContext context, CardView card, AlixTokens tokens) {
    if (_hasChoices) return _options(tokens);
    if (_state.mode == Mode.lineByLine && !_state.acquire) {
      return _revealLines(context, card.back.take(_revealedLines).toList(), tokens);
    }
    if (_isTyping && !_state.acquire) return _typing(context, card, tokens);
    if (_isExplain) return _explainBody(context, card, tokens);
    if (_revealed || _check != null) {
      return _revealLines(context, card.back, tokens);
    }
    return const SizedBox.shrink();
  }

  /// Revealed answer lines: monospace, neutral ink, centered; multi-line
  /// backs read as a stanza (a blank line between).
  Widget _revealLines(
      BuildContext context, List<String> lines, AlixTokens tokens) {
    final style = TextStyle(
      fontFamily: _mono,
      fontWeight: FontWeight.w500,
      fontSize: 18,
      height: 1.5,
      color: Theme.of(context).colorScheme.onSurface,
    );
    final stanza = lines.length > 1;
    return Column(
      children: [
        for (final (i, line) in lines.indexed) ...[
          if (i > 0) SizedBox(height: stanza ? 20 : 6),
          Text(line, textAlign: TextAlign.center, style: style),
        ],
      ],
    );
  }

  // ── multiple choice ──────────────────────────────────────────────────────

  Widget _options(AlixTokens tokens) {
    final options = _state.choices ?? const [];
    return Column(
      children: [
        for (final (i, opt) in options.indexed) ...[
          if (i > 0) const SizedBox(height: 10),
          _optionRow(i, opt, tokens),
        ],
      ],
    );
  }

  Widget _optionRow(int i, String opt, AlixTokens tokens) {
    final choice = _choice;
    final locked = choice != null;
    Color numColor = tokens.faint;
    Color textColor = tokens.text;
    Color borderColor = tokens.line;
    Color? fill = Colors.white.withValues(alpha: 0.03);
    double opacity = 1;
    if (locked) {
      final correct = BigInt.from(i) == choice.correct;
      final wrong = BigInt.from(i) == choice.chosen && !choice.passed;
      if (correct) {
        numColor = textColor = borderColor = tokens.good;
        fill = tokens.good.withValues(alpha: 0.12);
      } else if (wrong) {
        numColor = textColor = borderColor = tokens.again;
        fill = tokens.again.withValues(alpha: 0.13);
      } else {
        opacity = 0.45;
      }
    }
    final inner = Container(
      constraints: const BoxConstraints(minHeight: 52),
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 13),
      decoration: BoxDecoration(
        color: fill,
        border: Border.all(color: borderColor),
        borderRadius: BorderRadius.circular(12),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          Text('${i + 1}',
              style: TextStyle(fontFamily: _mono, fontSize: 13.5, color: numColor)),
          const SizedBox(width: 14),
          Expanded(
            child: Text(
              opt,
              style: TextStyle(
                  fontFamily: _mono, fontSize: 16, height: 1.35, color: textColor),
            ),
          ),
        ],
      ),
    );
    if (locked) {
      return Opacity(key: ValueKey('option-$i'), opacity: opacity, child: inner);
    }
    return Material(
      key: ValueKey('option-$i'),
      color: Colors.transparent,
      child: InkWell(
        borderRadius: BorderRadius.circular(12),
        onTap: () => setState(() => _choice = _session.choose(chosen: i)),
        child: inner,
      ),
    );
  }

  // ── typed answer ─────────────────────────────────────────────────────────

  Widget _typing(BuildContext context, CardView card, AlixTokens tokens) {
    final onSurface = Theme.of(context).colorScheme.onSurface;
    final fields = _state.mode == Mode.typeLine ? card.back.length : 1;
    OutlineInputBorder border(Color c) => OutlineInputBorder(
          borderRadius: BorderRadius.circular(12),
          borderSide: BorderSide(color: c),
        );
    return Column(
      children: [
        for (var i = 0; i < fields; i++) ...[
          if (i > 0) const SizedBox(height: 10),
          ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 520),
            child: TextField(
              controller: _typed[i],
              enabled: _check == null,
              textAlign: TextAlign.center,
              style: TextStyle(fontFamily: _mono, fontSize: 17, color: onSurface),
              decoration: InputDecoration(
                filled: true,
                fillColor: Colors.white.withValues(alpha: 0.04),
                contentPadding:
                    const EdgeInsets.symmetric(horizontal: 16, vertical: 13),
                enabledBorder: border(tokens.line),
                focusedBorder: border(tokens.bolt),
                border: border(tokens.line),
              ),
            ),
          ),
        ],
        if (_check != null) ...[
          const SizedBox(height: 14),
          for (final r in _check!.results) _evidenceLine(r, tokens),
        ],
      ],
    );
  }

  /// Per-line typed evidence: a passed line in green with a ✓; a miss recedes
  /// in dim red with a ✗ and the expected answer in green beneath it.
  Widget _evidenceLine(TypedResult r, AlixTokens tokens) {
    final input = r.input.isEmpty ? '(blank)' : r.input;
    if (r.passed) {
      return Padding(
        padding: const EdgeInsets.symmetric(vertical: 3),
        child: Text.rich(
          TextSpan(children: [
            TextSpan(
                text: input,
                style: TextStyle(
                    fontFamily: _mono,
                    fontWeight: FontWeight.w500,
                    fontSize: 18,
                    color: tokens.good)),
            TextSpan(
                text: '  ✓',
                style: TextStyle(
                    color: tokens.good, fontWeight: FontWeight.w700, fontSize: 18)),
          ]),
          textAlign: TextAlign.center,
        ),
      );
    }
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Column(
        children: [
          Text.rich(
            TextSpan(children: [
              TextSpan(
                  text: input,
                  style: TextStyle(
                      fontFamily: _mono,
                      fontSize: 15,
                      color: tokens.again.withValues(alpha: 0.5))),
              TextSpan(
                  text: '  ✗',
                  style: TextStyle(
                      color: tokens.again,
                      fontWeight: FontWeight.w700,
                      fontSize: 15)),
            ]),
            textAlign: TextAlign.center,
          ),
          Text(
            r.expected,
            textAlign: TextAlign.center,
            style: TextStyle(
                fontFamily: _mono,
                fontWeight: FontWeight.w500,
                fontSize: 18,
                color: tokens.good),
          ),
        ],
      ),
    );
  }

  void _submitCheck(CardView card) => setState(() {
        final fields = _state.mode == Mode.typeLine ? card.back.length : 1;
        _check = _session.check(
          lines: [for (var i = 0; i < fields; i++) _typed[i].text],
        );
      });

  // ── explain keypoint checklist ───────────────────────────────────────────

  Widget _explainBody(BuildContext context, CardView card, AlixTokens tokens) {
    if (!_revealed) {
      return Text(
        'reconstruct the answer in your head, then reveal.',
        textAlign: TextAlign.center,
        style: TextStyle(color: tokens.dim, fontSize: 14, fontStyle: FontStyle.italic),
      );
    }
    final points = _state.keypoints ?? const [];
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _explainLabel('the answer', tokens.dim),
        const SizedBox(height: 6),
        _revealLines(context, card.back, tokens),
        const SizedBox(height: 16),
        _explainLabel('did your answer cover these?', tokens.good),
        const SizedBox(height: 6),
        for (final (i, pt) in points.indexed) _keypointRow(i, pt, tokens),
      ],
    );
  }

  Widget _explainLabel(String text, Color color) {
    return Text(
      text.toUpperCase(),
      style: TextStyle(
        fontFamily: _mono,
        fontSize: 10.5,
        letterSpacing: 1.4,
        color: color,
      ),
    );
  }

  /// A keypoint row: tap to mark it covered (green ✓) or not (green ▸).
  Widget _keypointRow(int i, String pt, AlixTokens tokens) {
    final ticked = _ticked.contains(i);
    return InkWell(
      key: ValueKey('kp-$i'),
      borderRadius: BorderRadius.circular(7),
      onTap: () => setState(
          () => ticked ? _ticked.remove(i) : _ticked.add(i)),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 6, horizontal: 2),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
              width: 22,
              child: Text(ticked ? '✓' : '▸',
                  style: TextStyle(color: tokens.good, fontSize: 15, height: 1.45)),
            ),
            Expanded(
              child: Text(
                pt,
                style: TextStyle(
                  color: Theme.of(context).colorScheme.onSurface,
                  height: 1.45,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  // ── the note block ───────────────────────────────────────────────────────

  Widget _note(CardView card, AlixTokens tokens) {
    return Column(
      children: [
        const SizedBox(height: 18),
        _divider(tokens),
        const SizedBox(height: 14),
        Container(
          width: double.infinity,
          constraints: const BoxConstraints(maxWidth: 600),
          padding: const EdgeInsets.symmetric(horizontal: 15, vertical: 12),
          decoration: BoxDecoration(
            color: tokens.noteBorder.withValues(alpha: 0.12),
            border: Border.all(color: tokens.noteBorder.withValues(alpha: 0.24)),
            borderRadius: BorderRadius.circular(10),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              for (final (i, note) in card.note.indexed) ...[
                if (i > 0) const SizedBox(height: 10),
                switch (note) {
                  NoteUnit_Sentence(:final text) => Text(
                      text,
                      style: TextStyle(
                          color: tokens.noteInk, fontSize: 15, height: 1.4),
                    ),
                  NoteUnit_Code(:final lines) => Container(
                      width: double.infinity,
                      padding: const EdgeInsets.symmetric(
                          horizontal: 12, vertical: 10),
                      decoration: BoxDecoration(
                        color: Colors.black.withValues(alpha: 0.32),
                        borderRadius: BorderRadius.circular(8),
                      ),
                      child: Text(
                        lines.join('\n'),
                        style: TextStyle(
                            fontFamily: _mono,
                            fontSize: 13,
                            height: 1.45,
                            color: tokens.text),
                      ),
                    ),
                },
              ],
            ],
          ),
        ),
      ],
    );
  }

  // ── the footer legend (web chip parity) ──────────────────────────────────

  Widget _legend(BuildContext context, CardView card) {
    final chips = _legendChips(card);
    if (chips.isEmpty) {
      return SizedBox(height: 12 + MediaQuery.of(context).padding.bottom);
    }
    return Padding(
      padding: EdgeInsets.fromLTRB(
          12, 10, 12, 12 + MediaQuery.of(context).padding.bottom),
      child: Wrap(
        alignment: WrapAlignment.center,
        spacing: 10,
        runSpacing: 8,
        children: chips,
      ),
    );
  }

  /// The web's renderLegend() state matrix, mapped to chips. Keyboard hints
  /// are dropped (touch), and Ask-tutor/Skip await the AI-pairing milestone.
  List<Widget> _legendChips(CardView card) {
    // Acquire: reveal, then acknowledge with Seen.
    if (_state.acquire) {
      if (_hasChoices) {
        return _choice == null
            ? const []
            : [_chip('Seen', _ChipKind.primary, () => _apply(_session.acquire()))];
      }
      return _revealed
          ? [_chip('Seen', _ChipKind.primary, () => _apply(_session.acquire()))]
          : [_chip('Reveal', _ChipKind.primary, () => setState(() => _revealed = true))];
    }
    // Recognize multiple choice.
    if (_hasChoices) {
      final choice = _choice;
      if (choice == null) return const [];
      return choice.passed
          ? [
              _chip('Next', _ChipKind.primary, () => _grade(Grade.pass)),
              _chip('I guessed', _ChipKind.quiet, () => _grade(Grade.fail)),
            ]
          : [_chip('Continue', _ChipKind.primary, () => _grade(Grade.fail))];
    }
    // Line-by-line reveal.
    if (_state.mode == Mode.lineByLine) {
      if (!_lineDone(card)) {
        return [
          _chip(_revealedLines <= 1 ? 'Reveal' : 'Reveal next', _ChipKind.primary,
              () => setState(() => _revealedLines++)),
        ];
      }
      return _gradeTrio();
    }
    // Typed answer.
    if (_isTyping) {
      if (_check == null) {
        return [_chip('Submit', _ChipKind.primary, () => _submitCheck(card))];
      }
      return _gradeTrio();
    }
    // Explain checklist.
    if (_isExplain) {
      if (!_revealed) {
        return [_chip('Reveal', _ChipKind.primary, () => setState(() => _revealed = true))];
      }
      return [_verdictChip()];
    }
    // Flip / recognize-fallback: reveal, then grade.
    if (!_revealed) {
      return [_chip('Reveal', _ChipKind.primary, () => setState(() => _revealed = true))];
    }
    if (_state.depth == Depth.recognize) {
      return [
        _chip('Not yet', _ChipKind.failed, () => _grade(Grade.fail)),
        _chip('Knew it', _ChipKind.passed, () => _grade(Grade.pass)),
      ];
    }
    return _gradeTrio();
  }

  List<Widget> _gradeTrio() => [
        _chip('Missed it', _ChipKind.failed, () => _grade(Grade.fail)),
        _chip('Partly', _ChipKind.partly, () => _grade(Grade.partial)),
        _chip('Got it', _ChipKind.passed, () => _grade(Grade.pass)),
      ];

  /// The explain verdict chip: a filled chip showing the tick-tally's grade,
  /// committing it on tap (the web's Passed/Partly/Failed submit chip).
  Widget _verdictChip() {
    final tokens = Theme.of(context).alix;
    final grade = keypointGrade(
        covered: _ticked.length, total: (_state.keypoints ?? const []).length);
    final (label, color) = switch (grade) {
      Grade.fail => ('Failed', tokens.again),
      Grade.partial => ('Partly', tokens.warn),
      Grade.pass => ('Passed', tokens.good),
    };
    return _chip(label, _ChipKind.verdict, () => _grade(grade), verdictColor: color);
  }

  Widget _chip(String label, _ChipKind kind, VoidCallback? onTap,
      {Color? verdictColor}) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    Color? fill;
    Color borderColor = Colors.transparent;
    Color fg;
    FontWeight weight = FontWeight.w600;
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
      case _ChipKind.quiet:
        fg = tokens.dim;
        weight = FontWeight.w400;
      case _ChipKind.verdict:
        fill = verdictColor;
        borderColor = verdictColor ?? Colors.transparent;
        fg = theme.colorScheme.surface;
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
        // Content-width chips (like the web's inline-flex), so a grade trio
        // sits side by side. ~11px vertical padding gives the web's 42px.
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 15, vertical: 11),
          child: Text(
            label,
            style: TextStyle(
                fontFamily: _sans, fontWeight: weight, fontSize: 14, color: fg),
          ),
        ),
      ),
    );
  }

  // ── banners & done ───────────────────────────────────────────────────────

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

  /// The session-complete summary, mirroring the web's renderSummary().
  Widget _done(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    final reviews = _state.reviews;
    final acc = reviews > 0 ? '${(100 * _state.passed / reviews).round()}%' : '–';
    return SingleChildScrollView(
      padding: const EdgeInsets.all(24),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 12),
          Text(
            'SESSION COMPLETE',
            style: TextStyle(
                fontFamily: _mono,
                color: tokens.bolt,
                fontSize: 11,
                letterSpacing: 2.2),
          ),
          const SizedBox(height: 14),
          Text(
            reviews > 0 ? 'Nicely charged.' : 'Nothing due.',
            style: TextStyle(
                fontSize: 26,
                fontWeight: FontWeight.w600,
                color: theme.colorScheme.onSurface),
          ),
          const SizedBox(height: 18),
          _summaryRow('reviewed', '$reviews', tokens),
          _summaryRow('passed / failed', '${_state.passed} / ${_state.failed}', tokens),
          _summaryRow('accuracy', acc, tokens),
          if (!_state.canRestart) ...[
            const SizedBox(height: 18),
            Container(
              width: double.infinity,
              padding: const EdgeInsets.symmetric(horizontal: 15, vertical: 12),
              decoration: BoxDecoration(
                color: tokens.noteBorder.withValues(alpha: 0.12),
                border: Border.all(color: tokens.noteBorder.withValues(alpha: 0.24)),
                borderRadius: BorderRadius.circular(10),
              ),
              child: Text(
                'Nothing due right now, come back later.',
                style: TextStyle(color: tokens.noteInk, fontSize: 15),
              ),
            ),
          ],
          const SizedBox(height: 24),
          _chip(
            'New session',
            _state.canRestart ? _ChipKind.primary : _ChipKind.base,
            _state.canRestart ? () => setState(_open) : null,
          ),
        ],
      ),
    );
  }

  Widget _summaryRow(String label, String value, AlixTokens tokens) {
    return Container(
      padding: const EdgeInsets.symmetric(vertical: 9),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: tokens.line)),
      ),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Text(label, style: TextStyle(color: tokens.dim)),
          Text(
            value,
            style: TextStyle(
                fontFamily: _mono,
                fontWeight: FontWeight.w600,
                color: Theme.of(context).colorScheme.onSurface),
          ),
        ],
      ),
    );
  }
}

enum _ChipKind { base, primary, failed, partly, passed, quiet, verdict }
