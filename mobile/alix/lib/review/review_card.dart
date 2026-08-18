import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';

import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/review/sketch_canvas.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/shared/inline_runs.dart';
import 'package:alix_mobile/theme.dart';

const _mono = 'IBM Plex Mono';
const _sans = 'IBM Plex Sans';

class ReviewCardView extends StatelessWidget {
  const ReviewCardView({
    super.key,
    required this.state,
    required this.revealed,
    required this.revealedLines,
    required this.choice,
    required this.checkFeedback,
    required this.tickedKeypoints,
    required this.sketch,
    required this.onSketchBegin,
    required this.onSketchExtend,
    required this.onSketchEnd,
    required this.onSketchTool,
    required this.onSketchUndo,
    required this.onSketchClear,
    required this.attemptOpen,
    required this.attemptController,
    required this.typedControllers,
    required this.serverLive,
    required this.tutorCard,
    required this.verdictGrade,
    required this.onChoose,
    required this.onCheck,
    required this.onOpenAttempt,
    required this.onToggleKeypoint,
    required this.onReveal,
    required this.onRevealNextLine,
    required this.onIntroduce,
    required this.onGrade,
    required this.onOpenTutor,
  });

  final ReviewStateModel state;
  final bool revealed;
  final int revealedLines;
  final ReviewChoiceFeedbackModel? choice;
  final ReviewCheckFeedbackModel? checkFeedback;
  final Set<int> tickedKeypoints;
  final Sketch sketch;
  final void Function(Offset point, PointerDeviceKind kind) onSketchBegin;
  final ValueChanged<Offset> onSketchExtend;
  final VoidCallback onSketchEnd;
  final ValueChanged<SketchTool> onSketchTool;
  final VoidCallback onSketchUndo;
  final VoidCallback onSketchClear;
  final bool attemptOpen;
  final TextEditingController attemptController;
  final List<TextEditingController> typedControllers;
  final bool serverLive;
  final ReviewTutorCardModel? tutorCard;
  final ReviewGrade verdictGrade;
  final ValueChanged<int> onChoose;
  final ValueChanged<List<String>> onCheck;
  final VoidCallback onOpenAttempt;
  final ValueChanged<int> onToggleKeypoint;
  final VoidCallback onReveal;
  final VoidCallback onRevealNextLine;
  final VoidCallback onIntroduce;
  final ValueChanged<ReviewGrade> onGrade;
  final ValueChanged<ReviewTutorCardModel> onOpenTutor;

  bool get _hasChoices => state.choices?.isNotEmpty ?? false;
  bool get _isTyping =>
      state.mode == ReviewMode.typing || state.mode == ReviewMode.typeLine;
  bool get _isExplain =>
      state.mode == ReviewMode.explain &&
      !state.introducing &&
      state.keypoints != null;

  bool _lineDone(ReviewCardModel card) {
    return state.mode == ReviewMode.lineByLine &&
        revealedLines >= card.back.length;
  }

  @override
  Widget build(BuildContext context) {
    final card = state.card!;
    return Column(
      children: [
        Expanded(
          child: ScrollWithMoreHint(
            child: SingleChildScrollView(
              primary: true,
              padding: const EdgeInsets.fromLTRB(20, 8, 20, 8),
              child: SizedBox(
                width: double.infinity,
                child: _face(context, card),
              ),
            ),
          ),
        ),
        _legend(context, card),
      ],
    );
  }

  String _modeLabel() {
    if (state.introducing) return 'new';
    if (_hasChoices) return 'choice';
    return switch (state.mode) {
      ReviewMode.typeLine => 'typing · line',
      ReviewMode.typing => 'typing',
      ReviewMode.explain => 'explain',
      ReviewMode.lineByLine => 'line',
      ReviewMode.choice => 'choice',
      ReviewMode.flip => 'flip',
    };
  }

  Widget _face(BuildContext context, ReviewCardModel card) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    final answered =
        revealed || checkFeedback != null || choice != null || _lineDone(card);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        const SizedBox(height: 8),
        _modeTag(_modeLabel(), tokens),
        const SizedBox(height: 12),
        // A labelling context (a card table's title) sits above the prompt in
        // a quieter style; a leading one (a cloze sentence) is the question
        // itself and follows the front.
        if (!card.contextLeads)
          for (final (index, line) in card.context.indexed) ...[
            _runsOrText(
              index < card.contextRuns.length ? card.contextRuns[index] : null,
              line,
              textAlign: TextAlign.center,
              style: theme.textTheme.labelMedium?.copyWith(color: tokens.dim),
              contextHoles: false,
              tokens: tokens,
            ),
            const SizedBox(height: 8),
          ],
        _front(context, card, tokens),
        for (final image in card.images) ...[
          const SizedBox(height: 12),
          Image.file(File(image.src), height: 180, semanticLabel: image.alt),
        ],
        if (card.contextLeads)
          for (final (index, line) in card.context.indexed) ...[
            const SizedBox(height: 8),
            _runsOrText(
              index < card.contextRuns.length ? card.contextRuns[index] : null,
              line,
              textAlign: TextAlign.center,
              style: theme.textTheme.titleMedium?.copyWith(color: tokens.text),
              contextHoles: true,
              tokens: tokens,
            ),
          ],
        const SizedBox(height: 14),
        _divider(tokens),
        const SizedBox(height: 22),
        _body(context, card, tokens),
        if (answered)
          for (final image in card.imagesBack) ...[
            const SizedBox(height: 12),
            Image.file(File(image.src), height: 180, semanticLabel: image.alt),
          ],
        if (state.introducing && !_hasChoices && !revealed) ...[
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

  Widget _front(BuildContext context, ReviewCardModel card, AlixTokens tokens) {
    final style = TextStyle(
      fontFamily: _sans,
      fontWeight: FontWeight.w600,
      fontSize: 23,
      height: 1.4,
      color: Theme.of(context).colorScheme.onSurface,
    );
    final units = card.frontUnits;
    if (units == null) {
      return _runsOrText(
        card.frontRuns,
        card.front,
        style: style,
        textAlign: TextAlign.center,
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final (index, unit) in units.indexed) ...[
          if (index > 0) const SizedBox(height: 10),
          _unit(unit, tokens, style, TextAlign.center),
        ],
      ],
    );
  }

  Widget _runsOrText(
    List<InlineRunModel>? runs,
    String text, {
    required TextStyle? style,
    TextAlign textAlign = TextAlign.start,
    bool contextHoles = false,
    AlixTokens? tokens,
  }) {
    final effectiveStyle = style ?? const TextStyle();
    if (runs == null) {
      return Text(text, textAlign: textAlign, style: effectiveStyle);
    }
    return InlineRuns(
      runs: runs,
      style: effectiveStyle,
      textAlign: textAlign,
      contextHoles: contextHoles,
      holeColor: tokens?.boltHi,
      mutedHoleColor: tokens?.dim,
    );
  }

  Widget _unit(
    ReviewNoteUnitModel unit,
    AlixTokens tokens,
    TextStyle style,
    TextAlign textAlign,
  ) {
    return switch (unit) {
      ReviewSentenceModel(:final text, :final runs) => _runsOrText(
        runs,
        text,
        style: style,
        textAlign: textAlign,
      ),
      ReviewCodeModel(:final lines) => _codeBlock(
        lines,
        style.color ?? tokens.text,
      ),
      ReviewChecklistModel(:final items) => _checklist(items, tokens, style),
    };
  }

  Widget _codeBlock(List<String> lines, Color foreground) {
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
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
          color: foreground,
        ),
      ),
    );
  }

  Widget _checklist(
    List<ReviewChecklistItemModel> items,
    AlixTokens tokens,
    TextStyle style,
  ) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        for (final item in items)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 3),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  item.checked ? '☑' : '☐',
                  style: style.copyWith(
                    color: item.checked ? tokens.good : tokens.dim,
                  ),
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: _runsOrText(item.runs, item.text, style: style),
                ),
              ],
            ),
          ),
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

  Widget _divider(AlixTokens tokens) {
    return FractionallySizedBox(
      widthFactor: 0.7,
      child: Container(
        height: 1,
        decoration: BoxDecoration(
          gradient: LinearGradient(
            colors: [
              Colors.transparent,
              tokens.line,
              tokens.line,
              Colors.transparent,
            ],
            stops: const [0, 0.18, 0.82, 1],
          ),
        ),
      ),
    );
  }

  bool get _isDrawing => state.input == ReviewInput.draw;

  /// Above `_isTyping`: a draw card would otherwise take the typing branch and
  /// ask for the thing the deck says cannot be typed.
  Widget _sketchBody(BuildContext context, ReviewCardModel card, AlixTokens tokens) {
    final ink = Theme.of(context).colorScheme.onSurface;
    final canvas = DecoratedBox(
      decoration: BoxDecoration(
        border: Border.all(color: tokens.line),
        borderRadius: BorderRadius.circular(12),
      ),
      child: ClipRRect(
        borderRadius: BorderRadius.circular(12),
        child: SketchCanvas(
          sketch: sketch,
          ink: ink,
          frozen: revealed,
          onBegin: onSketchBegin,
          onExtend: onSketchExtend,
          onEnd: onSketchEnd,
        ),
      ),
    );
    // The answer region scrolls, so it hands down an unbounded height and the
    // canvas must claim a definite one. A share of the viewport rather than a
    // fixed number, so a tablet gets the room it has and a phone stays honest.
    final box = SizedBox(
      height: MediaQuery.sizeOf(context).height * 0.38,
      child: canvas,
    );
    if (revealed) {
      // The attempt stays on screen beside the answer: the sketch is not the
      // answer, it is what the learner grades themselves against.
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          box,
          const SizedBox(height: 12),
          _answerUnits(context, card.backUnits, tokens),
        ],
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        box,
        const SizedBox(height: 8),
        _sketchTools(tokens),
      ],
    );
  }

  Widget _sketchTools(AlixTokens tokens) {
    Widget tool(String label, VoidCallback onTap, {bool on = false}) {
      return Padding(
        padding: const EdgeInsets.only(right: 8),
        child: OutlinedButton(
          onPressed: onTap,
          style: OutlinedButton.styleFrom(
            side: BorderSide(color: on ? tokens.bolt : tokens.line),
            foregroundColor: on ? tokens.bolt : tokens.text,
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
            minimumSize: const Size(0, 36),
          ),
          child: Text(label, style: const TextStyle(fontFamily: _sans, fontSize: 13)),
        ),
      );
    }

    return Row(
      children: [
        tool('Pen', () => onSketchTool(SketchTool.pen), on: sketch.tool == SketchTool.pen),
        tool('Eraser', () => onSketchTool(SketchTool.eraser),
            on: sketch.tool == SketchTool.eraser),
        tool('Undo', onSketchUndo),
        tool('Clear', onSketchClear),
      ],
    );
  }

  Widget _body(BuildContext context, ReviewCardModel card, AlixTokens tokens) {
    if (_hasChoices) return _options(tokens);
    if (_isDrawing) return _sketchBody(context, card, tokens);
    if (state.mode == ReviewMode.lineByLine && (!state.introducing || revealed)) {
      final visible = state.introducing ? card.back.length : revealedLines;
      return _revealLines(
        context,
        card.back.take(visible).toList(),
        card.backRuns.take(visible).toList(),
        tokens,
        stanza: false,
      );
    }
    if (_isTyping && !state.introducing) return _typing(context, card, tokens);
    if (_isExplain) return _explainBody(context, card, tokens);
    if (revealed || checkFeedback != null) {
      if (!card.reshaped) {
        return _answerUnits(context, card.backUnits, tokens);
      }
      return _revealLines(context, card.back, card.backRuns, tokens);
    }
    return const SizedBox.shrink();
  }

  Widget _answerUnits(
    BuildContext context,
    List<ReviewNoteUnitModel> units,
    AlixTokens tokens,
  ) {
    final style = TextStyle(
      fontFamily: _mono,
      fontWeight: FontWeight.w500,
      fontSize: 18,
      height: 1.5,
      color: Theme.of(context).colorScheme.onSurface,
    );
    return Column(
      children: [
        for (final (index, unit) in units.indexed) ...[
          if (index > 0) const SizedBox(height: 10),
          _unit(unit, tokens, style, TextAlign.center),
        ],
      ],
    );
  }

  Widget _revealLines(
    BuildContext context,
    List<String> lines,
    List<List<InlineRunModel>> runLines,
    AlixTokens tokens, {
    bool stanza = true,
  }) {
    final style = TextStyle(
      fontFamily: _mono,
      fontWeight: FontWeight.w500,
      fontSize: 18,
      height: 1.5,
      color: Theme.of(context).colorScheme.onSurface,
    );
    final gap = stanza && lines.length > 1 ? 22.0 : 6.0;
    return Column(
      children: [
        for (final (index, line) in lines.indexed) ...[
          if (index > 0) SizedBox(height: gap),
          _runsOrText(
            index < runLines.length ? runLines[index] : null,
            line,
            textAlign: TextAlign.center,
            style: style,
          ),
        ],
      ],
    );
  }

  Widget _options(AlixTokens tokens) {
    final options = state.choices ?? const [];
    final optionRuns = state.choiceRuns;
    return Column(
      children: [
        for (final (index, option) in options.indexed) ...[
          if (index > 0) const SizedBox(height: 10),
          _optionRow(
            index,
            option,
            optionRuns != null && index < optionRuns.length
                ? optionRuns[index]
                : null,
            tokens,
          ),
        ],
      ],
    );
  }

  Widget _optionRow(
    int index,
    String option,
    List<InlineRunModel>? runs,
    AlixTokens tokens,
  ) {
    final feedback = choice;
    final locked = feedback != null;
    Color numberColor = tokens.faint;
    Color textColor = tokens.text;
    Color borderColor = tokens.line;
    Color? fill = Colors.white.withValues(alpha: 0.03);
    double opacity = 1;
    if (feedback != null) {
      final correct = index == feedback.correct;
      final wrong = index == feedback.chosen && !feedback.passed;
      if (correct) {
        numberColor = textColor = borderColor = tokens.good;
        fill = tokens.good.withValues(alpha: 0.12);
      } else if (wrong) {
        numberColor = textColor = borderColor = tokens.again;
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
          Text(
            '${index + 1}',
            style: TextStyle(
              fontFamily: _mono,
              fontSize: 13.5,
              color: numberColor,
            ),
          ),
          const SizedBox(width: 14),
          Expanded(
            child: _runsOrText(
              runs,
              option,
              style: TextStyle(
                fontFamily: _mono,
                fontSize: 16,
                height: 1.35,
                color: textColor,
              ),
            ),
          ),
        ],
      ),
    );
    if (locked) {
      return Opacity(
        key: ValueKey('option-$index'),
        opacity: opacity,
        child: inner,
      );
    }
    return Material(
      key: ValueKey('option-$index'),
      color: Colors.transparent,
      child: InkWell(
        borderRadius: BorderRadius.circular(12),
        onTap: () => onChoose(index),
        child: inner,
      ),
    );
  }

  Widget _typing(
    BuildContext context,
    ReviewCardModel card,
    AlixTokens tokens,
  ) {
    final feedback = checkFeedback;
    if (feedback != null) {
      return Column(
        children: [
          for (final result in feedback.results) _evidenceLine(result, tokens),
        ],
      );
    }
    final onSurface = Theme.of(context).colorScheme.onSurface;
    final fields = state.mode == ReviewMode.typeLine ? card.back.length : 1;
    OutlineInputBorder border(Color color) => OutlineInputBorder(
      borderRadius: BorderRadius.circular(12),
      borderSide: BorderSide(color: color),
    );
    return Column(
      children: [
        for (var index = 0; index < fields; index++) ...[
          if (index > 0) const SizedBox(height: 10),
          ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 520),
            child: TextField(
              controller: typedControllers[index],
              textAlign: TextAlign.center,
              style: TextStyle(
                fontFamily: _mono,
                fontSize: 17,
                color: onSurface,
              ),
              decoration: InputDecoration(
                filled: true,
                fillColor: Colors.white.withValues(alpha: 0.04),
                contentPadding: const EdgeInsets.symmetric(
                  horizontal: 16,
                  vertical: 13,
                ),
                enabledBorder: border(tokens.line),
                focusedBorder: border(tokens.bolt),
                border: border(tokens.line),
              ),
            ),
          ),
        ],
      ],
    );
  }

  Widget _evidenceLine(ReviewTypedResultModel result, AlixTokens tokens) {
    final input = result.input.isEmpty ? '(blank)' : result.input;
    if (result.passed) {
      return Padding(
        padding: const EdgeInsets.symmetric(vertical: 3),
        child: Text.rich(
          TextSpan(
            children: [
              TextSpan(
                text: input,
                style: TextStyle(
                  fontFamily: _mono,
                  fontWeight: FontWeight.w500,
                  fontSize: 18,
                  color: tokens.good,
                ),
              ),
              TextSpan(
                text: '  ✓',
                style: TextStyle(
                  color: tokens.good,
                  fontWeight: FontWeight.w700,
                  fontSize: 18,
                ),
              ),
            ],
          ),
          textAlign: TextAlign.center,
        ),
      );
    }
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Column(
        children: [
          Opacity(
            opacity: 0.5,
            child: Text.rich(
              TextSpan(
                children: [
                  TextSpan(
                    text: input,
                    style: TextStyle(
                      fontFamily: _mono,
                      fontSize: 15,
                      color: tokens.again,
                    ),
                  ),
                  TextSpan(
                    text: '  ✗',
                    style: TextStyle(
                      color: tokens.again,
                      fontWeight: FontWeight.w700,
                      fontSize: 15,
                    ),
                  ),
                ],
              ),
              textAlign: TextAlign.center,
            ),
          ),
          Text(
            result.expected,
            textAlign: TextAlign.center,
            style: TextStyle(
              fontFamily: _mono,
              fontWeight: FontWeight.w500,
              fontSize: 18,
              color: tokens.good,
            ),
          ),
        ],
      ),
    );
  }

  Widget _explainBody(
    BuildContext context,
    ReviewCardModel card,
    AlixTokens tokens,
  ) {
    if (!revealed) {
      if (!attemptOpen) {
        return Align(
          child: TextButton(
            onPressed: onOpenAttempt,
            child: const Text('type your answer first'),
          ),
        );
      }
      return ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 520),
        child: TextField(
          controller: attemptController,
          minLines: 2,
          maxLines: 5,
          decoration: InputDecoration(
            filled: true,
            fillColor: Colors.black.withValues(alpha: 0.25),
            border: OutlineInputBorder(borderRadius: BorderRadius.circular(12)),
            hintText: 'your answer (stays on this device)',
          ),
        ),
      );
    }
    final points = state.keypoints ?? const [];
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (attemptController.text.trim().isNotEmpty) ...[
          _explainLabel('your answer', tokens.dim),
          const SizedBox(height: 6),
          Text(
            attemptController.text.trim(),
            style: TextStyle(color: tokens.text, height: 1.4),
          ),
          const SizedBox(height: 16),
        ],
        _explainLabel('the answer', tokens.dim),
        const SizedBox(height: 6),
        _revealLines(context, card.back, card.backRuns, tokens, stanza: false),
        const SizedBox(height: 16),
        _explainLabel('did your answer cover these?', tokens.good, small: true),
        const SizedBox(height: 6),
        for (final (index, point) in points.indexed)
          _keypointRow(
            context,
            index,
            point,
            state.keypointRuns != null && index < state.keypointRuns!.length
                ? state.keypointRuns![index]
                : null,
            tokens,
          ),
      ],
    );
  }

  Widget _explainLabel(String text, Color color, {bool small = false}) {
    return Text(
      text.toUpperCase(),
      style: TextStyle(
        fontFamily: _mono,
        fontSize: small ? 9.5 : 10.5,
        letterSpacing: 1.4,
        color: color,
      ),
    );
  }

  Widget _keypointRow(
    BuildContext context,
    int index,
    String point,
    List<InlineRunModel>? runs,
    AlixTokens tokens,
  ) {
    final ticked = tickedKeypoints.contains(index);
    return InkWell(
      key: ValueKey('kp-$index'),
      borderRadius: BorderRadius.circular(7),
      onTap: () => onToggleKeypoint(index),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 6, horizontal: 2),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
              width: 22,
              child: Text(
                ticked ? '✓' : '▸',
                style: TextStyle(
                  color: tokens.good,
                  fontSize: 15,
                  height: 1.45,
                ),
              ),
            ),
            Expanded(
              child: _runsOrText(
                runs,
                point,
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

  Widget _note(ReviewCardModel card, AlixTokens tokens) {
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
            border: Border.all(
              color: tokens.noteBorder.withValues(alpha: 0.24),
            ),
            borderRadius: BorderRadius.circular(10),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              for (final (index, note) in card.note.indexed) ...[
                if (index > 0) const SizedBox(height: 10),
                switch (note) {
                  ReviewSentenceModel(:final text, :final runs) => _runsOrText(
                    runs,
                    text,
                    style: TextStyle(
                      color: tokens.noteInk,
                      fontSize: 15,
                      height: 1.4,
                    ),
                  ),
                  ReviewCodeModel(:final lines) => _codeBlock(
                    lines,
                    tokens.text,
                  ),
                  ReviewChecklistModel(:final items) => _checklist(
                    items,
                    tokens,
                    TextStyle(color: tokens.noteInk, fontSize: 15, height: 1.4),
                  ),
                },
              ],
            ],
          ),
        ),
      ],
    );
  }

  Widget _legend(BuildContext context, ReviewCardModel card) {
    final chips = _legendChips(card);
    if (chips.isEmpty) {
      return SizedBox(height: 12 + MediaQuery.of(context).padding.bottom);
    }
    return Padding(
      padding: EdgeInsets.fromLTRB(
        12,
        10,
        12,
        12 + MediaQuery.of(context).padding.bottom,
      ),
      child: Wrap(
        alignment: WrapAlignment.center,
        spacing: 10,
        runSpacing: 8,
        children: chips,
      ),
    );
  }

  bool _attempted(ReviewCardModel card) {
    if (_hasChoices) return choice != null;
    if (state.introducing) return revealed;
    if (state.mode == ReviewMode.lineByLine) return _lineDone(card);
    if (_isTyping) return checkFeedback != null;
    return revealed;
  }

  List<Widget> _legendChips(ReviewCardModel card) {
    final chips = [..._modeChips(card)];
    final tutor = tutorCard;
    if (serverLive && tutor != null && _attempted(card)) {
      chips.add(
        ReviewChip(
          label: 'Ask',
          kind: ReviewChipKind.quiet,
          onTap: () => onOpenTutor(tutor),
        ),
      );
    }
    return chips;
  }

  List<Widget> _modeChips(ReviewCardModel card) {
    if (state.introducing) {
      if (_hasChoices) {
        return choice == null
            ? const []
            : [
                ReviewChip(
                  label: 'Seen',
                  kind: ReviewChipKind.primary,
                  onTap: onIntroduce,
                ),
              ];
      }
      return revealed
          ? [
              ReviewChip(
                label: 'Seen',
                kind: ReviewChipKind.primary,
                onTap: onIntroduce,
              ),
            ]
          : [
              ReviewChip(
                label: 'Reveal',
                kind: ReviewChipKind.primary,
                onTap: onReveal,
              ),
            ];
    }
    if (_hasChoices) {
      final feedback = choice;
      if (feedback == null) return const [];
      return feedback.passed
          ? [
              ReviewChip(
                label: 'Next',
                kind: ReviewChipKind.primary,
                onTap: () => onGrade(ReviewGrade.pass),
              ),
              ReviewChip(
                label: 'I guessed',
                kind: ReviewChipKind.quiet,
                onTap: () => onGrade(ReviewGrade.fail),
              ),
            ]
          : [
              ReviewChip(
                label: 'Continue',
                kind: ReviewChipKind.primary,
                onTap: () => onGrade(ReviewGrade.fail),
              ),
            ];
    }
    if (state.mode == ReviewMode.lineByLine) {
      if (!_lineDone(card)) {
        return [
          ReviewChip(
            label: revealedLines == 0 ? 'Reveal' : 'Reveal next',
            kind: ReviewChipKind.primary,
            onTap: onRevealNextLine,
          ),
        ];
      }
      return _gradeTrio();
    }
    if (_isTyping) {
      if (checkFeedback == null) {
        final label = state.mode == ReviewMode.typeLine ? 'Check' : 'Submit';
        return [
          ReviewChip(
            label: label,
            kind: ReviewChipKind.primary,
            onTap: () {
              final fields = state.mode == ReviewMode.typeLine
                  ? card.back.length
                  : 1;
              onCheck([
                for (var index = 0; index < fields; index++)
                  typedControllers[index].text,
              ]);
            },
          ),
        ];
      }
      return _gradeTrio();
    }
    if (_isExplain) {
      if (!revealed) {
        return [
          ReviewChip(
            label: 'Reveal',
            kind: ReviewChipKind.primary,
            onTap: onReveal,
          ),
        ];
      }
      return [_verdictChip()];
    }
    if (!revealed) {
      return [
        ReviewChip(
          label: 'Reveal',
          kind: ReviewChipKind.primary,
          onTap: onReveal,
        ),
      ];
    }
    if (state.depth == ReviewDepth.recognize) {
      return [
        ReviewChip(
          label: 'Knew it',
          kind: ReviewChipKind.passed,
          onTap: () => onGrade(ReviewGrade.pass),
        ),
        ReviewChip(
          label: 'Not yet',
          kind: ReviewChipKind.failed,
          onTap: () => onGrade(ReviewGrade.fail),
        ),
      ];
    }
    return _gradeTrio();
  }

  List<Widget> _gradeTrio() => [
    ReviewChip(
      label: 'Missed it',
      kind: ReviewChipKind.failed,
      onTap: () => onGrade(ReviewGrade.fail),
    ),
    ReviewChip(
      label: 'Partly',
      kind: ReviewChipKind.partly,
      onTap: () => onGrade(ReviewGrade.partial),
    ),
    ReviewChip(
      label: 'Got it',
      kind: ReviewChipKind.passed,
      onTap: () => onGrade(ReviewGrade.pass),
    ),
  ];

  Widget _verdictChip() {
    final (label, kind) = switch (verdictGrade) {
      ReviewGrade.fail => ('Failed', ReviewChipKind.failedVerdict),
      ReviewGrade.partial => ('Partly', ReviewChipKind.partialVerdict),
      ReviewGrade.pass => ('Passed', ReviewChipKind.passedVerdict),
    };
    return ReviewChip(
      label: label,
      kind: kind,
      onTap: () => onGrade(verdictGrade),
    );
  }
}

enum ReviewChipKind {
  base,
  primary,
  failed,
  partly,
  passed,
  quiet,
  failedVerdict,
  partialVerdict,
  passedVerdict,
}

class ReviewChip extends StatelessWidget {
  const ReviewChip({
    super.key,
    required this.label,
    required this.kind,
    this.onTap,
  });

  final String label;
  final ReviewChipKind kind;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    Color? fill;
    Color borderColor = Colors.transparent;
    late final Color foreground;
    FontWeight weight = FontWeight.w600;
    switch (kind) {
      case ReviewChipKind.base:
        borderColor = tokens.line;
        foreground = tokens.text;
      case ReviewChipKind.primary:
        fill = theme.colorScheme.primary;
        borderColor = theme.colorScheme.primary;
        foreground = theme.colorScheme.onPrimary;
      case ReviewChipKind.failed:
        fill = tokens.again.withValues(alpha: 0.12);
        borderColor = tokens.again.withValues(alpha: 0.42);
        foreground = tokens.again;
      case ReviewChipKind.partly:
        fill = tokens.warn.withValues(alpha: 0.14);
        borderColor = tokens.warn.withValues(alpha: 0.42);
        foreground = tokens.warn;
      case ReviewChipKind.passed:
        fill = tokens.good.withValues(alpha: 0.13);
        borderColor = tokens.good.withValues(alpha: 0.42);
        foreground = tokens.good;
      case ReviewChipKind.quiet:
        foreground = tokens.dim;
        weight = FontWeight.w400;
      case ReviewChipKind.failedVerdict:
        fill = tokens.again;
        borderColor = tokens.again;
        foreground = theme.colorScheme.surface;
      case ReviewChipKind.partialVerdict:
        fill = tokens.warn;
        borderColor = tokens.warn;
        foreground = theme.colorScheme.surface;
      case ReviewChipKind.passedVerdict:
        fill = tokens.good;
        borderColor = tokens.good;
        foreground = theme.colorScheme.surface;
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
            style: TextStyle(
              fontFamily: _sans,
              fontWeight: weight,
              fontSize: 14,
              color: foreground,
            ),
          ),
        ),
      ),
    );
  }
}

/// Overlays a quiet "more" pill while its scrollable child extends below the
/// viewport, mirroring the web client's "more below" hint. The pill reacts to
/// content growth too (a reveal adding the note), via metrics notifications,
/// and disappears once the reader reaches the bottom.
class ScrollWithMoreHint extends StatefulWidget {
  const ScrollWithMoreHint({super.key, required this.child});

  final Widget child;

  @override
  State<ScrollWithMoreHint> createState() => _ScrollWithMoreHintState();
}

class _ScrollWithMoreHintState extends State<ScrollWithMoreHint> {
  final ScrollController _controller = ScrollController();
  bool _moreBelow = false;

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _update(ScrollMetrics metrics) {
    final more = metrics.hasContentDimensions &&
        metrics.maxScrollExtent > 4 &&
        metrics.pixels < metrics.maxScrollExtent - 4;
    if (more != _moreBelow) setState(() => _moreBelow = more);
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return PrimaryScrollController(
      controller: _controller,
      child: NotificationListener<ScrollMetricsNotification>(
        onNotification: (n) {
          _update(n.metrics);
          return false;
        },
        child: NotificationListener<ScrollNotification>(
          onNotification: (n) {
            _update(n.metrics);
            return false;
          },
          child: Stack(
            children: [
              widget.child,
              Positioned(
                left: 0,
                right: 0,
                bottom: 6,
                child: IgnorePointer(
                  child: AnimatedOpacity(
                    opacity: _moreBelow ? 1 : 0,
                    duration: const Duration(milliseconds: 150),
                    child: Center(
                      child: Container(
                        padding: const EdgeInsets.symmetric(
                            horizontal: 10, vertical: 3),
                        decoration: BoxDecoration(
                          color: theme.colorScheme.surface.withValues(alpha: 0.9),
                          borderRadius: BorderRadius.circular(10),
                          border: Border.all(color: theme.dividerColor),
                        ),
                        child: Text(
                          '⌵ more',
                          style: TextStyle(
                            fontSize: 12,
                            color: theme.alix.faint,
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
