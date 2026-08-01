import 'package:flutter/material.dart';

import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/shared/inline_runs.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/walk/walk_models.dart';
import 'package:alix_mobile/walk/walk_widgets.dart';

const _mono = 'IBM Plex Mono';

class WalkSummaryView extends StatelessWidget {
  const WalkSummaryView({
    super.key,
    required this.state,
    required this.examAvailable,
    required this.cooldownMs,
    required this.onOpenExam,
    required this.onRestart,
  });

  final WalkStateModel state;
  final bool examAvailable;
  final int? cooldownMs;
  final VoidCallback onOpenExam;
  final VoidCallback onRestart;

  @override
  Widget build(BuildContext context) {
    final tokens = Theme.of(context).alix;
    final summary =
        state.summary ??
        WalkSummaryModel(
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
            'WALK COMPLETE',
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
                      text: 'Trace walked.',
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
          if (examAvailable) ...[
            WalkChip(
              label: 'Take the exam',
              kind: WalkChipKind.primary,
              onTap: onOpenExam,
            ),
            const SizedBox(height: 12),
          ],
          WalkChip(
            label: 'Walk again',
            kind: examAvailable ? WalkChipKind.base : WalkChipKind.primary,
            onTap: onRestart,
          ),
          if (cooldownMs case final cooldown?) ...[
            const SizedBox(height: 12),
            Text(
              'Walk the trace again before re-sitting; '
              '${_humanizeCooldown(cooldown)} left.',
              style: TextStyle(color: tokens.dim, fontSize: 13),
            ),
          ],
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

  String _humanizeCooldown(int ms) {
    final minutes = (ms / 60000).ceil();
    return minutes <= 1 ? 'about a minute' : 'about ${minutes}m';
  }
}
