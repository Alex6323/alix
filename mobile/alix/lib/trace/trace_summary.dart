import 'package:flutter/material.dart';

import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/shared/inline_runs.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/trace/trace_models.dart';
import 'package:alix_mobile/trace/trace_widgets.dart';

const _mono = 'IBM Plex Mono';

class TraceSessionSummaryView extends StatelessWidget {
  const TraceSessionSummaryView({
    super.key,
    required this.state,
    required this.onRestart,
  });

  final TraceSessionStateModel state;
  final VoidCallback onRestart;

  @override
  Widget build(BuildContext context) {
    final tokens = Theme.of(context).alix;
    final summary =
        state.summary ??
        TraceSessionSummaryModel(
          passed: 0,
          partly: 0,
          failed: 0,
          weak: const [],
          total: 0,
        );
    return SingleChildScrollView(
      padding: const EdgeInsets.all(24),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 12),
          Text(
            'TRACE COMPLETE',
            style: TextStyle(
              fontFamily: _mono,
              color: tokens.bolt,
              fontSize: 11,
              letterSpacing: 2.2,
            ),
          ),
          const SizedBox(height: 14),
          InlineRuns(
            runs: state.description.isEmpty
                ? [
                    const InlineRunModel(
                      text: 'Trace finished.',
                      bold: false,
                      italic: false,
                      code: false,
                    ),
                  ]
                : state.descriptionRuns,
            style: TextStyle(
              fontSize: 24,
              fontWeight: FontWeight.w600,
              color: Theme.of(context).colorScheme.onSurface,
            ),
          ),
          const SizedBox(height: 18),
          _summaryRow(
            context,
            'got it',
            '${summary.passed}',
            tokens,
            valueColor: tokens.good,
          ),
          _summaryRow(
            context,
            'partly',
            '${summary.partly}',
            tokens,
            valueColor: tokens.warn,
          ),
          _summaryRow(
            context,
            'missed it',
            '${summary.failed}',
            tokens,
            valueColor: tokens.again,
          ),
          if (summary.weak.isNotEmpty)
            _summaryRow(
              context,
              'weak (resurface sooner)',
              summary.weak.map((hop) => '#$hop').join(' · '),
              tokens,
            )
          else if (summary.total > 0)
            _summaryRow(
              context,
              'every checkpoint landed',
              '✓',
              tokens,
              valueColor: tokens.good,
            ),
          const SizedBox(height: 24),
          TraceSessionChip(
            label: 'Trace again',
            kind: TraceSessionChipKind.primary,
            onTap: onRestart,
          ),
        ],
      ),
    );
  }

  Widget _summaryRow(
    BuildContext context,
    String label,
    String value,
    AlixTokens tokens, {
    Color? valueColor,
  }) {
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
              color: valueColor ?? Theme.of(context).colorScheme.onSurface,
            ),
          ),
        ],
      ),
    );
  }
}
