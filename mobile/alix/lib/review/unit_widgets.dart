// The content-unit renderers shared by the review card and the tutor sheet:
// one widget per `ReviewContentUnitModel` kind, the same on both surfaces so
// a tutor answer reads like the card it explains.
import 'dart:io';

import 'package:flutter/material.dart';

import 'package:alix_mobile/review/masked_image.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/shared/inline_runs.dart';
import 'package:alix_mobile/theme.dart';

const monoFontFamily = 'IBM Plex Mono';

Widget unitWidget(
  ReviewContentUnitModel unit,
  AlixTokens tokens,
  TextStyle style,
  TextAlign textAlign,
) {
  return switch (unit) {
    ReviewSentenceModel(:final text, :final runs) => runsOrText(
      runs,
      text,
      style: style,
      textAlign: textAlign,
    ),
    ReviewCodeModel(:final lines) => codeBlock(
      lines,
      style.color ?? tokens.text,
    ),
    ReviewDiagramModel() => diagramWidget(unit, answered: true),
    ReviewChecklistModel(:final items) => checklistWidget(
      items,
      tokens,
      style,
      textAlign == TextAlign.center ? TextAlign.start : textAlign,
    ),
    ReviewTableModel() => tableWidget(unit, tokens, style),
    ReviewQuoteModel(:final units) => quoteWidget(units, tokens, style, textAlign),
  };
}

/// A quoted block, its own units stacked behind a rule. `surface` is the
/// alignment of what the quotation sits in, not of its own prose.
Widget quoteWidget(
  List<ReviewContentUnitModel> units,
  AlixTokens tokens,
  TextStyle style,
  TextAlign surface,
) {
  return Container(
    padding: const EdgeInsets.only(left: 12),
    decoration: BoxDecoration(
      border: Border(left: BorderSide(color: tokens.dim, width: 3)),
    ),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        for (final unit in units)
          unitWidget(
            unit,
            tokens,
            style,
            unit is ReviewSentenceModel ? TextAlign.start : surface,
          ),
      ],
    ),
  );
}

Widget tableWidget(ReviewTableModel unit, AlixTokens tokens, TextStyle style) {
  TextAlign cellAlign(int index) => switch (index < unit.aligns.length
      ? unit.aligns[index]
      : ReviewCellAlign.none) {
    ReviewCellAlign.center => TextAlign.center,
    ReviewCellAlign.right => TextAlign.right,
    ReviewCellAlign.none || ReviewCellAlign.left => TextAlign.left,
  };
  Widget cell(List<InlineRunModel> runs, int index, TextStyle cellStyle) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 5),
      child: runsOrText(
        runs,
        '',
        style: cellStyle,
        textAlign: cellAlign(index),
      ),
    );
  }

  return SingleChildScrollView(
    scrollDirection: Axis.horizontal,
    child: Table(
      defaultColumnWidth: const IntrinsicColumnWidth(),
      border: TableBorder.all(color: tokens.dim.withValues(alpha: 0.4)),
      children: [
        TableRow(
          children: [
            for (final (index, runs) in unit.header.indexed)
              cell(runs, index, style.copyWith(fontWeight: FontWeight.w600)),
          ],
        ),
        for (final row in unit.rows)
          TableRow(
            children: [
              for (final (index, runs) in row.indexed)
                cell(runs, index, style),
            ],
          ),
      ],
    ),
  );
}

Widget checklistWidget(
  List<ReviewChecklistItemModel> items,
  AlixTokens tokens,
  TextStyle style,
  TextAlign textAlign,
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
                child: runsOrText(
                  item.runs,
                  item.text,
                  style: style,
                  textAlign: textAlign,
                ),
              ),
            ],
          ),
        ),
    ],
  );
}

Widget runsOrText(
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

Widget diagramWidget(ReviewDiagramModel unit, {required bool answered}) {
  final alt = answered && unit.revealedAlt != null
      ? unit.revealedAlt!
      : unit.alt;
  if (unit.regions.isEmpty) {
    return Semantics(
      label: alt,
      image: true,
      child: Image.file(
        File(unit.src),
        width: unit.width.toDouble(),
        fit: BoxFit.scaleDown,
        errorBuilder: (_, _, _) => const SizedBox.shrink(),
      ),
    );
  }
  // A masked diagram is the shipped occlusion surface over the frozen
  // raster: regions are raster-pixel boxes, exactly what MaskedCardImage
  // places against the decoded source size.
  return Semantics(
    label: alt,
    image: true,
    child: Builder(
      builder: (context) => MaskedCardImage(
        provider: FileImage(File(unit.src)),
        image: ReviewImageModel(
          src: unit.src,
          alt: alt,
          regions: unit.regions,
          crop: null,
        ),
        answered: answered,
        height: unit.height.toDouble(),
        onAskedGone: () => ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(
            content: Text(
              'this card asks about a region outside its diagram',
            ),
          ),
        ),
      ),
    ),
  );
}

Widget codeBlock(List<String> lines, Color foreground) {
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
        fontFamily: monoFontFamily,
        fontSize: 13,
        height: 1.45,
        color: foreground,
      ),
    ),
  );
}
