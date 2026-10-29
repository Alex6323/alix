import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';

import 'package:alix_mobile/review/masked_image.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/review/sketch_canvas.dart';
import 'package:alix_mobile/review/unit_widgets.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/shared/veil.dart';
import 'package:alix_mobile/theme.dart';

const _sans = 'IBM Plex Sans';

// One measure for the whole card, so the question, the answer and the note
// wrap on the same column instead of each finding its own.
const _cardMeasure = 560.0;

/// A note is prose dimmer than the answer, marked by a hairline in its badge's
/// colour with the badge word above the text: no box, no wash, nothing that
/// competes with the answer for attention.
class NoteBlock extends StatelessWidget {
  const NoteBlock({super.key, required this.badge, required this.children});

  final ReviewBadge badge;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final accent = badgeColour(badge, Theme.of(context).brightness);
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.fromLTRB(17, 2, 0, 2),
      decoration: BoxDecoration(
        border: Border(left: BorderSide(color: accent, width: 1)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            badge.name.toUpperCase(),
            style: TextStyle(
              fontFamily: monoFontFamily,
              fontSize: 10.5,
              height: 1.5,
              letterSpacing: 1.7,
              fontWeight: FontWeight.w600,
              color: accent,
            ),
          ),
          const SizedBox(height: 6),
          ...children,
        ],
      ),
    );
  }
}

/// GitHub's badge colours, the light set on a light theme and the dark set on
/// a dark one, the same on every palette.
Color badgeColour(ReviewBadge badge, Brightness brightness) {
  final light = brightness == Brightness.light;
  return switch (badge) {
    ReviewBadge.note => light
        ? const Color(0xFF0969DA)
        : const Color(0xFF4493F8),
    ReviewBadge.tip => light ? const Color(0xFF1A7F37) : const Color(0xFF3FB950),
    ReviewBadge.important => light
        ? const Color(0xFF8250DF)
        : const Color(0xFFAB7DF8),
    ReviewBadge.warning => light
        ? const Color(0xFF9A6700)
        : const Color(0xFFD29922),
    ReviewBadge.caution => light
        ? const Color(0xFFCF222E)
        : const Color(0xFFF85149),
  };
}

class ReviewCardView extends StatelessWidget {
  const ReviewCardView({
    super.key,
    required this.state,
    required this.revealed,
    required this.revealedLines,
    required this.choice,
    required this.multiChoice,
    required this.multiSelected,
    required this.checkFeedback,
    required this.typelineChecked,
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
    required this.onToggleChoice,
    required this.onSubmitChoices,
    required this.onCheck,
    required this.onOpenAttempt,
    required this.onToggleKeypoint,
    required this.onReveal,
    required this.onRevealNextLine,
    required this.onIntroduce,
    required this.onGrade,
    required this.onOpenTutor,
    required this.onOpenSection,
    this.onWalkNext,
    this.answerHidden = false,
    this.onToggleHidden,
  });

  final ReviewStateModel state;
  final bool revealed;
  final int revealedLines;
  final ReviewChoiceFeedbackModel? choice;
  final ReviewMultiChoiceFeedbackModel? multiChoice;
  final Set<int> multiSelected;
  final ReviewCheckFeedbackModel? checkFeedback;

  /// TypeLine's accepted prefix, rendered above the one current field.
  final List<ReviewTypedResultModel> typelineChecked;
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
  final ValueChanged<int> onToggleChoice;
  final VoidCallback onSubmitChoices;
  final ValueChanged<List<String>> onCheck;
  final VoidCallback onOpenAttempt;
  final ValueChanged<int> onToggleKeypoint;
  final VoidCallback onReveal;
  final VoidCallback onRevealNextLine;
  final VoidCallback onIntroduce;
  final ValueChanged<ReviewGrade> onGrade;
  final ValueChanged<ReviewTutorCardModel> onOpenTutor;
  final ValueChanged<ReviewCardModel> onOpenSection;
  final VoidCallback? onWalkNext;

  /// The self-test veil over a revealed answer; a tap on the answer region
  /// flips it through [onToggleHidden].
  final bool answerHidden;
  final VoidCallback? onToggleHidden;

  bool get _hasChoices => state.choices?.isNotEmpty ?? false;
  bool get _isMulti => state.choicesMultiple == true;
  bool get _isTyping =>
      state.mode == ReviewMode.typing || state.mode == ReviewMode.typeLine;
  bool get _isExplain =>
      state.mode == ReviewMode.explain &&
      !state.introducing &&
      state.keypoints != null;

  bool _lineDone(ReviewCardModel card) {
    return state.mode == ReviewMode.lineByLine &&
        revealedLines >= revealStepCount(card);
  }

  @override
  Widget build(BuildContext context) {
    final card = state.card!;
    final tokens = Theme.of(context).alix;
    final answered = _answered(card);
    final showNote = answered && card.note.isNotEmpty;
    final hideable = _hideable(card);
    final hidden = hideable && answerHidden;
    return Column(
      children: [
        Expanded(
          child: LayoutBuilder(
            builder: (context, box) {
              final body = box.maxHeight;
              return Column(
                key: const ValueKey('card-body'),
                children: [
                  ConstrainedBox(
                    constraints: BoxConstraints(maxHeight: body * 0.38),
                    child: _region(
                      key: const ValueKey('question-region'),
                      padding: const EdgeInsets.fromLTRB(20, 8, 20, 14),
                      child: _head(context, card, tokens, answered: answered),
                    ),
                  ),
                  _measured(_divider(tokens)),
                  Expanded(
                    child: _AnswerNoteLayout(
                      floor: body / 3,
                      answer: _answerTap(
                        hideable,
                        VeilScope(
                          veiled: hidden,
                          child: _region(
                            key: const ValueKey('answer-region'),
                            contentKey: const ValueKey('answer-content'),
                            padding: const EdgeInsets.fromLTRB(20, 22, 20, 8),
                            child: _answer(
                              context,
                              card,
                              tokens,
                              answered: answered,
                            ),
                          ),
                        ),
                      ),
                      note: showNote
                          ? _region(
                              key: const ValueKey('note-region'),
                              contentKey: const ValueKey('note-content'),
                              padding: const EdgeInsets.fromLTRB(20, 12, 20, 8),
                              child: Visibility(
                                visible: !hidden,
                                maintainSize: true,
                                maintainState: true,
                                maintainAnimation: true,
                                child: _note(context, card, tokens),
                              ),
                            )
                          : null,
                    ),
                  ),
                ],
              );
            },
          ),
        ),
        _legend(context, card),
      ],
    );
  }

  bool _answered(ReviewCardModel card) =>
      revealed ||
      checkFeedback != null ||
      choice != null ||
      multiChoice != null ||
      _lineDone(card);

  /// Every revealed text answer can be hidden for a self-test; a sketch, a
  /// choice pick and a typed check have nothing to veil.
  bool _hideable(ReviewCardModel card) =>
      (revealed || revealedLines > 0) &&
      !_hasChoices &&
      !_isDrawing &&
      checkFeedback == null;

  Widget _answerTap(bool hideable, Widget region) {
    final toggle = onToggleHidden;
    if (!hideable || toggle == null) return region;
    return GestureDetector(
      key: const ValueKey('answer-tap'),
      behavior: HitTestBehavior.opaque,
      onTap: toggle,
      child: region,
    );
  }

  /// One scrolling region of the card, its content on the reading measure.
  Widget _region({
    required Key key,
    Key? contentKey,
    required EdgeInsets padding,
    required Widget child,
  }) {
    return KeyedSubtree(
      key: key,
      child: ScrollWithMoreHint(
        child: SingleChildScrollView(
          primary: true,
          child: _measured(
            Padding(key: contentKey, padding: padding, child: child),
          ),
        ),
      ),
    );
  }

  Widget _measured(Widget child) {
    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: _cardMeasure),
        child: SizedBox(width: double.infinity, child: child),
      ),
    );
  }

  Widget _head(
    BuildContext context,
    ReviewCardModel card,
    AlixTokens tokens, {
    required bool answered,
  }) {
    final theme = Theme.of(context);
    final rectsLifted =
        answered ||
        questionAt(
          card,
          filledFrontSteps(
            card,
            mode: state.mode,
            introducing: state.introducing,
            answered: answered,
            revealedLines: revealedLines,
          ),
        ).rectsRevealed;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        ..._contextLines(
          card,
          style: theme.textTheme.labelMedium?.copyWith(color: tokens.dim),
        ),
        _questionTap(card, _front(context, card, tokens, answered: answered)),
        for (final image in card.images) ...[
          const SizedBox(height: 12),
          _cardImage(context, image, rectsLifted),
        ],
      ],
    );
  }

  Widget _answer(
    BuildContext context,
    ReviewCardModel card,
    AlixTokens tokens, {
    required bool answered,
  }) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        _body(context, card, tokens),
        if (answered)
          for (final image in card.imagesBack) ...[
            const SizedBox(height: 12),
            VeilInvisible(child: _cardImage(context, image, answered)),
          ],
      ],
    );
  }

  Widget _questionTap(ReviewCardModel card, Widget front) {
    if (!card.hasSection) return front;
    return GestureDetector(
      key: const ValueKey('question-tap'),
      behavior: HitTestBehavior.opaque,
      onTap: () => onOpenSection(card),
      child: SizedBox(width: double.infinity, child: Center(child: front)),
    );
  }

  Widget _cardImage(
    BuildContext context,
    ReviewImageModel image,
    bool answered,
  ) {
    if (image.regions.isEmpty && image.crop == null) {
      return Image.file(File(image.src), height: 180, semanticLabel: image.alt);
    }
    return MaskedCardImage(
      provider: FileImage(File(image.src)),
      image: image,
      answered: answered,
      height: 180,
      onAskedGone: () => ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(
          content: Text(
            'a blank lies outside the image, so its question cannot be drawn',
          ),
        ),
      ),
    );
  }

  Widget _front(
    BuildContext context,
    ReviewCardModel card,
    AlixTokens tokens, {
    required bool answered,
  }) {
    final style = TextStyle(
      fontFamily: _sans,
      fontWeight: FontWeight.w600,
      fontSize: 23,
      height: 1.4,
      color: Theme.of(context).colorScheme.onSurface,
    );
    final question = questionAt(
      card,
      filledFrontSteps(
        card,
        mode: state.mode,
        introducing: state.introducing,
        answered: answered,
        revealedLines: revealedLines,
      ),
    );
    final units = question.frontUnits;
    if (units == null) {
      return runsOrText(
        question.frontRuns,
        question.front,
        style: style,
        contextHoles: true,
        tokens: tokens,
      );
    }
    return Column(
      crossAxisAlignment: _blockAlignment(units),
      children: [
        for (final (index, unit) in units.indexed) ...[
          if (index > 0) const SizedBox(height: 10),
          switch (unit) {
            ReviewDiagramModel() => diagramWidget(unit, answered: answered),
            ReviewSentenceModel(:final text, :final runs) => runsOrText(
              runs,
              text,
              style: style,
              contextHoles: true,
              tokens: tokens,
            ),
            ReviewCodeModel(:final lines) => codeBlock(
              lines,
              style.color ?? tokens.text,
              blankTokens: tokens,
            ),
            _ => unitWidget(unit, tokens, style, TextAlign.start),
          },
        ],
      ],
    );
  }

  /// Prose shrinks to its widest line and the region centres it; a block
  /// holding anything else spans the measure, as on the web.
  CrossAxisAlignment _blockAlignment(Iterable<ReviewContentUnitModel> units) =>
      units.every((unit) => unit is ReviewSentenceModel)
      ? CrossAxisAlignment.start
      : CrossAxisAlignment.stretch;

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
  Widget _sketchBody(
    BuildContext context,
    ReviewCardModel card,
    AlixTokens tokens,
  ) {
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
    if (revealed && card.frontSteps.isNotEmpty) {
      return box;
    }
    if (revealed) {
      // The attempt stays on screen beside the answer: the sketch is not the
      // answer, it is what the learner grades themselves against.
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          box,
          const SizedBox(height: 12),
          Center(child: _answerUnits(context, card.backUnits, tokens)),
        ],
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [box, const SizedBox(height: 8), _sketchTools(tokens)],
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
          child: Text(
            label,
            style: const TextStyle(fontFamily: _sans, fontSize: 13),
          ),
        ),
      );
    }

    return Row(
      children: [
        tool(
          'Pen',
          () => onSketchTool(SketchTool.pen),
          on: sketch.tool == SketchTool.pen,
        ),
        tool(
          'Eraser',
          () => onSketchTool(SketchTool.eraser),
          on: sketch.tool == SketchTool.eraser,
        ),
        tool('Undo', onSketchUndo),
        tool('Clear', onSketchClear),
      ],
    );
  }

  Widget _body(BuildContext context, ReviewCardModel card, AlixTokens tokens) {
    if (_hasChoices) return _options(tokens);
    if (_isDrawing) return _sketchBody(context, card, tokens);
    final answerInQuestion = card.frontSteps.isNotEmpty;
    if (state.mode == ReviewMode.lineByLine &&
        (!state.introducing || revealed)) {
      if (answerInQuestion) return const SizedBox.shrink();
      final visible = state.introducing
          ? card.answerSteps.length
          : revealedLines;
      return _revealSteps(context, card, visible, tokens);
    }
    if (_isTyping && !state.introducing) return _typing(context, card, tokens);
    if (_isExplain) return _explainBody(context, card, tokens);
    if (revealed || checkFeedback != null) {
      if (answerInQuestion) return const SizedBox.shrink();
      if (!card.reshaped) {
        return _answerUnits(context, card.backUnits, tokens);
      }
      return _fullAnswer(context, card, tokens, stanza: true);
    }
    return const SizedBox.shrink();
  }

  /// Every surface that shows a WHOLE answer walks the same steps the
  /// progressive reveal does, so a block step renders as its block on all of
  /// them and not only on the one path that was built for it.
  ///
  /// `stanza` keys on PHYSICAL lines, not steps: a reshaped answer earns its
  /// wider gap from how many lines the author wrote, and a lone two-line
  /// quotation is one step over two of them.
  Widget _fullAnswer(
    BuildContext context,
    ReviewCardModel card,
    AlixTokens tokens, {
    bool stanza = false,
  }) => _revealSteps(
    context,
    card,
    card.answerSteps.length,
    tokens,
    gap: stanza && card.back.length > 1 ? 22 : 6,
  );

  Widget _answerUnits(
    BuildContext context,
    List<ReviewContentUnitModel> units,
    AlixTokens tokens,
  ) {
    final style = TextStyle(
      fontWeight: FontWeight.w500,
      fontSize: 18,
      height: 1.5,
      color: Theme.of(context).colorScheme.onSurface,
    );
    return Column(
      crossAxisAlignment: _blockAlignment(units),
      children: [
        for (final (index, unit) in units.indexed) ...[
          if (index > 0) const SizedBox(height: 10),
          unitWidget(unit, tokens, style, TextAlign.start),
        ],
      ],
    );
  }

  /// Line reveal walks the answer's STEPS, not its physical lines: a
  /// quotation or a table is one step and reveals whole, as its own block
  /// rather than as raw markers.
  Widget _revealSteps(
    BuildContext context,
    ReviewCardModel card,
    int visible,
    AlixTokens tokens, {
    double gap = 6,
  }) {
    final style = TextStyle(
      fontWeight: FontWeight.w500,
      fontSize: 18,
      height: 1.5,
      color: Theme.of(context).colorScheme.onSurface,
    );
    final children = <Widget>[];
    void addGap() {
      if (children.isNotEmpty) children.add(SizedBox(height: gap));
    }

    var fenceIndex = 0;
    var index = 0;
    var prose = true;
    while (index < visible && index < card.answerSteps.length) {
      final step = card.answerSteps[index];
      if (step is ReviewAnswerQuoteModel) {
        addGap();
        children.add(quoteWidget(step.units, tokens, style, TextAlign.start));
        prose = false;
        index++;
        continue;
      }
      if (step is ReviewAnswerTableModel) {
        addGap();
        for (final unit in step.units) {
          if (unit is ReviewTableModel) {
            children.add(tableWidget(unit, tokens, style));
          }
        }
        prose = false;
        index++;
        continue;
      }
      final from = step.backFrom;
      var to = step.backTo;
      index++;
      while (index < visible &&
          index < card.answerSteps.length &&
          card.answerSteps[index] is ReviewAnswerLineModel) {
        to = card.answerSteps[index].backTo;
        index++;
      }
      final lines = card.back.sublist(from, to);
      final runLines = card.backRuns.sublist(from, to);
      fenceIndex = _walkFences(
        lines,
        card.backUnits,
        fenceStart: fenceIndex,
        onFence: (code, unit, closed) {
          addGap();
          prose = false;
          if (closed && unit is ReviewDiagramModel) {
            children.add(diagramWidget(unit, answered: true));
          } else {
            children.add(codeBlock(code, style.color ?? tokens.text));
          }
        },
        onLine: (line) {
          addGap();
          children.add(
            runsOrText(
              line < runLines.length ? runLines[line] : null,
              lines[line],
              style: style,
            ),
          );
        },
      );
    }
    return Column(
      crossAxisAlignment: prose
          ? CrossAxisAlignment.start
          : CrossAxisAlignment.stretch,
      children: children,
    );
  }

  List<Widget> _contextLines(ReviewCardModel card, {TextStyle? style}) {
    return [
      for (final (index, line) in card.context.indexed) ...[
        runsOrText(
          index < card.contextRuns.length ? card.contextRuns[index] : null,
          line,
          textAlign: TextAlign.center,
          style: style,
        ),
        const SizedBox(height: 8),
      ],
    ];
  }

  // The ONE fence walk (the same alignment law as the web clients):
  // fence-shaped units arrive in the same document order as the raw fences,
  // the nth fence consumes the nth unit, and a resolved diagram replaces
  // its fence only once the closing marker is within the walked lines; a
  // partial fence stays a code block.
  /// `fenceStart` lets a caller render an answer in several passes (a block
  /// step splits it) without a later pass pairing a fence with an earlier
  /// unit; it returns where the next pass resumes.
  int _walkFences(
    List<String> lines,
    List<ReviewContentUnitModel> units, {
    required void Function(
      List<String> code,
      ReviewContentUnitModel? unit,
      bool closed,
    )
    onFence,
    required void Function(int index) onLine,
    int fenceStart = 0,
  }) {
    final fenceUnits = [
      for (final unit in units)
        if (unit is ReviewCodeModel || unit is ReviewDiagramModel) unit,
    ];
    var fenceIndex = fenceStart;
    var index = 0;
    while (index < lines.length) {
      final opener = _fenceOpener(lines[index]);
      if (opener != null) {
        final code = <String>[];
        index++;
        while (index < lines.length && !_closesFence(lines[index], opener)) {
          code.add(lines[index]);
          index++;
        }
        final closed = index < lines.length;
        if (closed) index++;
        final unit = fenceIndex < fenceUnits.length
            ? fenceUnits[fenceIndex]
            : null;
        fenceIndex++;
        onFence(code, unit, closed);
        continue;
      }
      onLine(index);
      index++;
    }
    return fenceIndex;
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
    if (_isMulti) return _multiOptionRow(index, option, runs, tokens);
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
    return _optionTile(
      index: index,
      option: option,
      runs: runs,
      tokens: tokens,
      numberColor: numberColor,
      textColor: textColor,
      borderColor: borderColor,
      fill: fill,
      opacity: opacity,
      locked: locked,
      onTap: () => onChoose(index),
    );
  }

  Widget _multiOptionRow(
    int index,
    String option,
    List<InlineRunModel>? runs,
    AlixTokens tokens,
  ) {
    final feedback = multiChoice;
    final locked = feedback != null;
    Color numberColor = tokens.faint;
    Color textColor = tokens.text;
    Color borderColor = tokens.line;
    Color? fill = Colors.white.withValues(alpha: 0.03);
    double opacity = 1;
    if (feedback != null) {
      final correct = feedback.correct.contains(index);
      final picked = feedback.chosen.contains(index);
      if (correct) {
        numberColor = textColor = borderColor = tokens.good;
        if (picked) fill = tokens.good.withValues(alpha: 0.12);
      } else if (picked) {
        numberColor = textColor = borderColor = tokens.again;
        fill = tokens.again.withValues(alpha: 0.13);
      } else {
        opacity = 0.45;
      }
    } else if (multiSelected.contains(index)) {
      numberColor = borderColor = tokens.text;
      fill = Colors.white.withValues(alpha: 0.1);
    }
    return _optionTile(
      index: index,
      option: option,
      runs: runs,
      tokens: tokens,
      numberColor: numberColor,
      textColor: textColor,
      borderColor: borderColor,
      fill: fill,
      opacity: opacity,
      locked: locked,
      onTap: () => onToggleChoice(index),
    );
  }

  Widget _optionTile({
    required int index,
    required String option,
    required List<InlineRunModel>? runs,
    required AlixTokens tokens,
    required Color numberColor,
    required Color textColor,
    required Color borderColor,
    required Color? fill,
    required double opacity,
    required bool locked,
    required VoidCallback onTap,
  }) {
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
              fontFamily: monoFontFamily,
              fontSize: 13.5,
              color: numberColor,
            ),
          ),
          const SizedBox(width: 14),
          Expanded(
            child: runsOrText(
              runs,
              option,
              style: TextStyle(
                fontFamily: monoFontFamily,
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
        onTap: onTap,
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
    // TypeLine reconstructs one gradeable step at a time: the checked prefix
    // stands above, and exactly one field is open. Ordinary Typing is one
    // shot, so it opens every gradeable field at once.
    final progressive = state.mode == ReviewMode.typeLine;
    final fields = progressive ? 1 : card.gradeableSteps.length;
    OutlineInputBorder border(Color color) => OutlineInputBorder(
      borderRadius: BorderRadius.circular(12),
      borderSide: BorderSide(color: color),
    );
    return Column(
      children: [
        for (final result in typelineChecked) _evidenceLine(result, tokens),
        if (progressive && typelineChecked.isNotEmpty)
          const SizedBox(height: 10),
        for (var index = 0; index < fields; index++) ...[
          if (index > 0) const SizedBox(height: 10),
          ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: _cardMeasure),
            child: TextField(
              controller: typedControllers[index],
              textAlign: TextAlign.center,
              style: TextStyle(
                fontFamily: monoFontFamily,
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
                  fontFamily: monoFontFamily,
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
                      fontFamily: monoFontFamily,
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
              fontFamily: monoFontFamily,
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
        constraints: const BoxConstraints(maxWidth: _cardMeasure),
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
        _fullAnswer(context, card, tokens),
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
        fontFamily: monoFontFamily,
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
              child: runsOrText(
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

  /// No hairline between the answer and the notes: each note's bar and badge
  /// word are the separation.
  Widget _note(
    BuildContext context,
    ReviewCardModel card,
    AlixTokens tokens,
  ) {
    return Column(
      children: [
        for (final (index, note) in card.note.indexed) ...[
          if (index > 0) const SizedBox(height: 10),
          _noteBlock(context, note, tokens),
        ],
      ],
    );
  }

  // One block per note, so several notes stack instead of merging into one
  // and losing their badges.
  Widget _noteBlock(
    BuildContext context,
    ReviewNoteModel entry,
    AlixTokens tokens,
  ) {
    final body = TextStyle(color: tokens.dim, fontSize: 15, height: 1.4);
    return NoteBlock(
      badge: entry.badge,
      children: [
        for (final (index, note) in entry.units.indexed) ...[
          if (index > 0) const SizedBox(height: 10),
          switch (note) {
            ReviewSentenceModel(:final text, :final runs) => runsOrText(
              runs,
              text,
              style: body,
            ),
            ReviewCodeModel(:final lines) => codeBlock(lines, tokens.text),
            ReviewDiagramModel() => diagramWidget(note, answered: true),
            ReviewChecklistModel(:final items) => checklistWidget(
              items,
              tokens,
              body,
              TextAlign.start,
            ),
            ReviewTableModel() => tableWidget(note, tokens, body),
            ReviewQuoteModel(:final units) => quoteWidget(
              units,
              tokens,
              body,
              TextAlign.start,
            ),
          },
        ],
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
    if (_hasChoices) return _isMulti ? multiChoice != null : choice != null;
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
    if (onWalkNext case final next? when _attempted(card)) {
      return [
        ReviewChip(label: 'Next', kind: ReviewChipKind.primary, onTap: next),
      ];
    }
    if (state.introducing) {
      if (_hasChoices) {
        if (_isMulti) {
          return multiChoice == null
              ? [
                  ReviewChip(
                    label: 'Submit',
                    kind: ReviewChipKind.primary,
                    onTap: onSubmitChoices,
                  ),
                ]
              : [
                  ReviewChip(
                    label: 'Seen',
                    kind: ReviewChipKind.primary,
                    onTap: onIntroduce,
                  ),
                ];
        }
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
      if (_isMulti) {
        final feedback = multiChoice;
        if (feedback == null) {
          return [
            ReviewChip(
              label: 'Submit',
              kind: ReviewChipKind.primary,
              onTap: onSubmitChoices,
            ),
          ];
        }
        return _choiceVerdictChips(feedback.passed);
      }
      final feedback = choice;
      if (feedback == null) return const [];
      return _choiceVerdictChips(feedback.passed);
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
              if (state.mode == ReviewMode.typeLine) {
                onCheck([typedControllers[0].text]);
                return;
              }
              onCheck([
                for (var index = 0;
                    index < card.gradeableSteps.length;
                    index++)
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

  List<Widget> _choiceVerdictChips(bool passed) {
    return passed
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

String? _fenceOpener(String line) {
  final trimmed = line.trim();
  final ch = trimmed.startsWith('```')
      ? '`'
      : trimmed.startsWith('~~~')
      ? '~'
      : null;
  if (ch == null) return null;
  var run = 3;
  while (run < trimmed.length && trimmed[run] == ch) {
    run++;
  }
  return trimmed.substring(0, run);
}

bool _closesFence(String line, String opener) {
  final trimmed = line.trim();
  return trimmed.length >= opener.length &&
      trimmed.codeUnits.every((unit) => unit == opener.codeUnitAt(0));
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

/// The room under the divider, shared by the answer and the note: the note
/// takes what it needs out of the space the answer is not using, and the
/// answer yields only down to [floor] when its own content needs more.
class _AnswerNoteLayout extends MultiChildRenderObjectWidget {
  _AnswerNoteLayout({
    required this.floor,
    required Widget answer,
    Widget? note,
  }) : super(children: [answer, ?note]);

  final double floor;

  @override
  RenderObject createRenderObject(BuildContext context) =>
      _RenderAnswerNote(floor);

  @override
  void updateRenderObject(
    BuildContext context,
    _RenderAnswerNote renderObject,
  ) {
    renderObject.floor = floor;
  }
}

class _AnswerNoteParentData extends ContainerBoxParentData<RenderBox> {}

class _RenderAnswerNote extends RenderBox
    with
        ContainerRenderObjectMixin<RenderBox, _AnswerNoteParentData>,
        RenderBoxContainerDefaultsMixin<RenderBox, _AnswerNoteParentData> {
  _RenderAnswerNote(this._floor);

  double _floor;
  set floor(double value) {
    if (value == _floor) return;
    _floor = value;
    markNeedsLayout();
  }

  @override
  void setupParentData(RenderBox child) {
    if (child.parentData is! _AnswerNoteParentData) {
      child.parentData = _AnswerNoteParentData();
    }
  }

  // Each child is laid out loose first to learn what it needs, then tight
  // to its share; a dry layout would do, but the content holds a
  // LayoutBuilder, which cannot be dry-laid.
  @override
  void performLayout() {
    final width = constraints.maxWidth;
    final rest = constraints.maxHeight;
    final answer = firstChild!;
    final note = childAfter(answer);
    final loose = BoxConstraints(
      minWidth: width,
      maxWidth: width,
      maxHeight: rest,
    );
    var noteHeight = 0.0;
    if (note != null) {
      answer.layout(loose, parentUsesSize: true);
      note.layout(loose, parentUsesSize: true);
      final answerFloor = answer.size.height.clamp(0.0, _floor);
      final room = (rest - answerFloor).clamp(0.0, rest);
      noteHeight = note.size.height.clamp(0.0, room);
    }
    final answerHeight = rest - noteHeight;
    answer.layout(BoxConstraints.tight(Size(width, answerHeight)));
    (answer.parentData! as _AnswerNoteParentData).offset = Offset.zero;
    if (note != null) {
      note.layout(BoxConstraints.tight(Size(width, noteHeight)));
      (note.parentData! as _AnswerNoteParentData).offset = Offset(
        0,
        answerHeight,
      );
    }
    size = Size(width, rest);
  }

  @override
  void paint(PaintingContext context, Offset offset) =>
      defaultPaint(context, offset);

  @override
  bool hitTestChildren(BoxHitTestResult result, {required Offset position}) =>
      defaultHitTestChildren(result, position: position);
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
    final more =
        metrics.hasContentDimensions &&
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
                          horizontal: 10,
                          vertical: 3,
                        ),
                        decoration: BoxDecoration(
                          color: theme.colorScheme.surface.withValues(
                            alpha: 0.9,
                          ),
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

void _walkContextBlocks(
  List<String> lines,
  List<ReviewContentUnitModel> units, {
  required void Function(
    List<String> source,
    ReviewContentUnitModel? unit,
    bool closed,
  )
  onBlock,
  required void Function(int index) onLine,
}) {
  var unitIndex = 0;
  var index = 0;
  while (index < lines.length) {
    if (lines[index].trim() == r'$$') {
      var close = index + 1;
      while (close < lines.length && lines[close].trim() != r'$$') {
        close++;
      }
      if (close < lines.length) {
        final source = lines.sublist(index + 1, close);
        if (source.any((line) => line.trim().isNotEmpty)) {
          final unit = unitIndex < units.length ? units[unitIndex] : null;
          unitIndex++;
          onBlock(source, unit, true);
        }
        index = close + 1;
        continue;
      }
    }

    final opener = _fenceOpener(lines[index]);
    if (opener != null) {
      final source = <String>[];
      index++;
      while (index < lines.length && !_closesFence(lines[index], opener)) {
        source.add(lines[index]);
        index++;
      }
      final closed = index < lines.length;
      if (closed) index++;
      final unit = unitIndex < units.length ? units[unitIndex] : null;
      unitIndex++;
      onBlock(source, unit, closed);
      continue;
    }

    onLine(index);
    index++;
  }
}

TextStyle _sectionProseStyle(ThemeData theme, AlixTokens tokens) {
  return theme.textTheme.bodyMedium?.copyWith(color: tokens.text, height: 1.45) ??
      TextStyle(color: tokens.text, height: 1.45);
}

/// The section's prose after its heading (line 0): consecutive plain lines
/// are one paragraph (a hard wrap is a space, as in Markdown), a blank line
/// ends it, a closed fence is a code block, a closed `$$` block its diagram,
/// the labelling context's walk.
List<Widget> _sectionProse(
  ReviewCardModel card,
  AlixTokens tokens,
  TextStyle style,
) {
  final children = <Widget>[];
  void add(Widget widget) {
    children.add(const SizedBox(height: 8));
    children.add(widget);
  }

  final textParts = <String>[];
  final runParts = <List<InlineRunModel>?>[];
  void flush() {
    if (textParts.isEmpty) return;
    final runs = runParts.contains(null)
        ? null
        : _joinRunLines(runParts.cast<List<InlineRunModel>>());
    add(runsOrText(runs, textParts.join(' '), style: style));
    textParts.clear();
    runParts.clear();
  }

  final body = card.section.isEmpty ? const <String>[] : card.section.sublist(1);
  _walkContextBlocks(
    body,
    card.sectionUnits,
    onBlock: (source, unit, closed) {
      flush();
      if (closed && unit is ReviewDiagramModel) {
        add(diagramWidget(unit, answered: true));
      } else if (closed && unit is ReviewSentenceModel) {
        add(runsOrText(unit.runs, unit.text, style: style));
      } else {
        add(codeBlock(source, style.color ?? tokens.text));
      }
    },
    onLine: (index) {
      final at = index + 1;
      final line = card.section[at];
      if (line.trim().isEmpty) {
        flush();
        return;
      }
      textParts.add(line);
      runParts.add(
        at < card.sectionRuns.length ? card.sectionRuns[at] : null,
      );
    },
  );
  flush();
  return children;
}

List<InlineRunModel> _joinRunLines(List<List<InlineRunModel>> lines) {
  const space = InlineRunModel(
    text: ' ',
    bold: false,
    italic: false,
    code: false,
  );
  return [
    for (final (index, runs) in lines.indexed) ...[
      if (index > 0) space,
      ...runs,
    ],
  ];
}

Future<void> showSectionSheet(BuildContext context, ReviewCardModel card) {
  return showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    useSafeArea: true,
    builder: (_) => SectionSheet(card: card),
  );
}

/// The card's section on demand: a modal bottom sheet with the heading as
/// its title, a close mark in the top-right corner, and the prose below,
/// the tutor sheet's shape.
class SectionSheet extends StatelessWidget {
  const SectionSheet({super.key, required this.card});

  final ReviewCardModel card;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(20, 8, 8, 20),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Expanded(
                  child: Padding(
                    padding: const EdgeInsets.only(top: 12),
                    child: Text(
                      card.sectionTitle,
                      key: const ValueKey('section-sheet-title'),
                      style: theme.textTheme.titleMedium,
                    ),
                  ),
                ),
                IconButton(
                  key: const ValueKey('section-sheet-close'),
                  onPressed: () => Navigator.of(context).pop(),
                  icon: Icon(
                    Icons.close,
                    color: tokens.dim,
                    semanticLabel: 'Close',
                  ),
                ),
              ],
            ),
            Flexible(
              child: SingleChildScrollView(
                padding: const EdgeInsets.only(right: 12),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: _sectionProse(
                    card,
                    tokens,
                    _sectionProseStyle(theme, tokens),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
