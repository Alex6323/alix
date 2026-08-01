import 'package:flutter/material.dart';

import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/shared/inline_runs.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/walk/walk_models.dart';

const _mono = 'IBM Plex Mono';
const _sans = 'IBM Plex Sans';

class WalkSaveBanner extends StatelessWidget {
  const WalkSaveBanner({super.key, required this.saveError});

  final String saveError;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Container(
      margin: const EdgeInsets.fromLTRB(12, 8, 12, 0),
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
      decoration: BoxDecoration(
        color: theme.colorScheme.errorContainer,
        borderRadius: BorderRadius.circular(8),
      ),
      child: Tooltip(
        message: saveError,
        child: Text(
          "Progress isn't being saved. Grades stay on screen and save with "
          'the next successful one.',
          style: theme.textTheme.bodySmall?.copyWith(
            color: theme.colorScheme.onErrorContainer,
          ),
        ),
      ),
    );
  }
}

class WalkDescriptionEyebrow extends StatelessWidget {
  const WalkDescriptionEyebrow({super.key, required this.state});

  final WalkStateModel state;

  @override
  Widget build(BuildContext context) {
    if (state.description.isEmpty) return const SizedBox.shrink();
    final tokens = Theme.of(context).alix;
    return Padding(
      padding: const EdgeInsets.fromLTRB(20, 6, 20, 0),
      child: Align(
        alignment: Alignment.centerLeft,
        child: InlineRuns(
          runs: state.descriptionRuns,
          style: TextStyle(
            fontFamily: _mono,
            fontSize: 11.5,
            letterSpacing: 0.4,
            color: tokens.bolt,
          ),
        ),
      ),
    );
  }
}

class WalkPhaseBody extends StatelessWidget {
  const WalkPhaseBody({
    super.key,
    required this.state,
    required this.predictionController,
  });

  final WalkStateModel state;
  final TextEditingController predictionController;

  @override
  Widget build(BuildContext context) {
    final tokens = Theme.of(context).alix;
    return SingleChildScrollView(
      padding: const EdgeInsets.fromLTRB(20, 8, 20, 8),
      child: SizedBox(
        width: double.infinity,
        child: state.phase == WalkPhaseModel.reveal
            ? _revealBody(context, tokens)
            : _predictBody(context, tokens),
      ),
    );
  }

  Widget _predictBody(BuildContext context, AlixTokens tokens) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        const SizedBox(height: 8),
        InlineRuns(
          runs: state.promptRuns ?? const [],
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
            controller: predictionController,
            minLines: 3,
            maxLines: 8,
            decoration: InputDecoration(
              filled: true,
              fillColor: Colors.black.withValues(alpha: 0.25),
              border: OutlineInputBorder(
                borderRadius: BorderRadius.circular(12),
              ),
              hintText:
                  'predict the next checkpoint, even a hunch beats nothing',
            ),
          ),
        ),
      ],
    );
  }

  Widget _givensRow(AlixTokens tokens) {
    if (state.givens.isEmpty) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.only(top: 10),
      child: Wrap(
        alignment: WrapAlignment.center,
        spacing: 8,
        runSpacing: 6,
        children: [for (final runs in state.givenRuns) _givenTag(runs, tokens)],
      ),
    );
  }

  Widget _givenTag(List<InlineRunModel> runs, AlixTokens tokens) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 5),
      decoration: BoxDecoration(
        border: Border.all(color: tokens.line),
        borderRadius: BorderRadius.circular(8),
      ),
      child: InlineRuns(
        runs: runs,
        style: TextStyle(fontFamily: _mono, fontSize: 12, color: tokens.dim),
      ),
    );
  }

  Widget _locatorLabel(AlixTokens tokens) {
    final locator = state.locator;
    if (locator == null) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.only(top: 8),
      child: Text(
        'at $locator',
        style: TextStyle(
          fontFamily: _mono,
          fontSize: 11.5,
          color: tokens.faint,
        ),
      ),
    );
  }

  Widget _revealBody(BuildContext context, AlixTokens tokens) {
    final onSurface = Theme.of(context).colorScheme.onSurface;
    final prediction = state.prediction;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        _label('you predicted', tokens.dim),
        const SizedBox(height: 4),
        Text(
          prediction ?? '(no prediction)',
          style: TextStyle(
            color: onSurface.withValues(alpha: prediction == null ? 0.5 : 0.8),
            height: 1.4,
            fontStyle: prediction == null ? FontStyle.italic : FontStyle.normal,
          ),
        ),
        const SizedBox(height: 16),
        _label('the source', tokens.bolt),
        _locatorLabel(tokens),
        const SizedBox(height: 6),
        _excerptBlock(tokens),
        if (state.points.isNotEmpty) ...[
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
      style: TextStyle(
        fontFamily: _mono,
        fontSize: 10.5,
        letterSpacing: 1.4,
        color: color,
      ),
    );
  }

  Widget _excerptBlock(AlixTokens tokens) {
    final excerpt = state.excerpt;
    if (excerpt == null) {
      return Container(
        width: double.infinity,
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
        decoration: BoxDecoration(
          border: Border.all(color: tokens.line),
          borderRadius: BorderRadius.circular(10),
        ),
        child: Text(
          state.excerptError ?? 'no excerpt for this checkpoint',
          style: TextStyle(
            color: tokens.dim,
            fontStyle: FontStyle.italic,
            fontSize: 13,
          ),
        ),
      );
    }
    final terms = _sourceTerms();
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
              style: TextStyle(
                fontFamily: _mono,
                fontSize: 11,
                color: tokens.dim,
              ),
            ),
          ),
          Divider(height: 1, color: tokens.line),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                for (final line in excerpt.lines)
                  _gutterLine(line, terms, tokens),
                if (excerpt.truncated)
                  Padding(
                    padding: const EdgeInsets.only(top: 4),
                    child: Text(
                      '… excerpt truncated',
                      style: TextStyle(
                        color: tokens.faint,
                        fontSize: 11,
                        fontStyle: FontStyle.italic,
                      ),
                    ),
                  ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  List<String> _sourceTerms() {
    final terms = <String>{};
    for (final runs in state.pointRuns) {
      for (final run in runs) {
        if (run.code && run.text.trim().isNotEmpty) terms.add(run.text);
      }
    }
    return terms.toList()..sort((left, right) {
      final byLength = right.length.compareTo(left.length);
      return byLength != 0 ? byLength : left.compareTo(right);
    });
  }

  List<TextSpan>? _sourceSpans(
    String text,
    List<String> terms,
    AlixTokens tokens,
  ) {
    final spans = <TextSpan>[];
    var cursor = 0;
    var matched = false;
    while (cursor < text.length) {
      var nextAt = -1;
      var nextTerm = '';
      for (final term in terms) {
        final at = text.indexOf(term, cursor);
        if (at >= 0 &&
            (nextAt < 0 ||
                at < nextAt ||
                (at == nextAt && term.length > nextTerm.length))) {
          nextAt = at;
          nextTerm = term;
        }
      }
      if (nextAt < 0) {
        spans.add(TextSpan(text: text.substring(cursor)));
        break;
      }
      if (nextAt > cursor) {
        spans.add(TextSpan(text: text.substring(cursor, nextAt)));
      }
      spans.add(
        TextSpan(
          text: nextTerm,
          style: TextStyle(
            color: tokens.bolt,
            fontWeight: FontWeight.w600,
            decoration: TextDecoration.underline,
            decorationColor: tokens.bolt.withValues(alpha: 0.55),
          ),
        ),
      );
      matched = true;
      cursor = nextAt + nextTerm.length;
    }
    return matched ? spans : null;
  }

  Widget _gutterLine(
    WalkLineModel line,
    List<String> terms,
    AlixTokens tokens,
  ) {
    final style = TextStyle(
      fontFamily: _mono,
      fontSize: 13,
      color: tokens.text,
    );
    final spans = _sourceSpans(line.text, terms, tokens);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 1),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 32,
            child: Text(
              '${line.number}',
              style: TextStyle(
                fontFamily: _mono,
                fontSize: 12,
                color: tokens.faint,
              ),
            ),
          ),
          Expanded(
            child: spans == null
                ? Text(line.text, style: style)
                : Text.rich(TextSpan(style: style, children: spans)),
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
        for (final runs in state.pointRuns)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 3),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                SizedBox(
                  width: 18,
                  child: Text(
                    '▸',
                    style: TextStyle(color: tokens.good, fontSize: 14),
                  ),
                ),
                Expanded(
                  child: InlineRuns(
                    runs: runs,
                    style: TextStyle(color: onSurface, height: 1.4),
                  ),
                ),
              ],
            ),
          ),
      ],
    );
  }

  Widget _noteBlock(AlixTokens tokens) {
    final note = state.note;
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
      child: InlineRuns(
        runs: state.noteRuns ?? const [],
        style: TextStyle(color: tokens.noteInk, fontSize: 15, height: 1.4),
      ),
    );
  }
}

class WalkFooter extends StatelessWidget {
  const WalkFooter({
    super.key,
    required this.phase,
    required this.onReveal,
    required this.onGrade,
  });

  final WalkPhaseModel phase;
  final VoidCallback onReveal;
  final ValueChanged<WalkGrade> onGrade;

  @override
  Widget build(BuildContext context) {
    final chips = switch (phase) {
      WalkPhaseModel.predict => [
        WalkChip(label: 'Reveal', kind: WalkChipKind.primary, onTap: onReveal),
      ],
      WalkPhaseModel.reveal => [
        WalkChip(
          label: 'Missed it',
          kind: WalkChipKind.failed,
          onTap: () => onGrade(WalkGrade.missed),
        ),
        WalkChip(
          label: 'Partly',
          kind: WalkChipKind.partly,
          onTap: () => onGrade(WalkGrade.partly),
        ),
        WalkChip(
          label: 'Got it',
          kind: WalkChipKind.passed,
          onTap: () => onGrade(WalkGrade.got),
        ),
      ],
      WalkPhaseModel.done => const <Widget>[],
    };
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
}

enum WalkChipKind { base, primary, failed, partly, passed }

class WalkChip extends StatelessWidget {
  const WalkChip({
    super.key,
    required this.label,
    required this.kind,
    this.onTap,
  });

  final String label;
  final WalkChipKind kind;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    Color? fill;
    Color borderColor = Colors.transparent;
    late final Color foreground;
    switch (kind) {
      case WalkChipKind.base:
        borderColor = tokens.line;
        foreground = tokens.text;
      case WalkChipKind.primary:
        fill = theme.colorScheme.primary;
        borderColor = theme.colorScheme.primary;
        foreground = theme.colorScheme.onPrimary;
      case WalkChipKind.failed:
        fill = tokens.again.withValues(alpha: 0.12);
        borderColor = tokens.again.withValues(alpha: 0.42);
        foreground = tokens.again;
      case WalkChipKind.partly:
        fill = tokens.warn.withValues(alpha: 0.14);
        borderColor = tokens.warn.withValues(alpha: 0.42);
        foreground = tokens.warn;
      case WalkChipKind.passed:
        fill = tokens.good.withValues(alpha: 0.13);
        borderColor = tokens.good.withValues(alpha: 0.42);
        foreground = tokens.good;
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
              fontWeight: FontWeight.w600,
              fontSize: 14,
              color: foreground,
            ),
          ),
        ),
      ),
    );
  }
}
