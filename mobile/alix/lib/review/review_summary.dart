import 'package:flutter/material.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/theme.dart';

const _mono = 'IBM Plex Mono';

class ReviewSummaryView extends StatelessWidget {
  const ReviewSummaryView({
    super.key,
    required this.state,
    required this.nowMs,
    required this.examAvailable,
    required this.onRestart,
    required this.onOpenExam,
  });

  final ReviewStateModel state;
  final int nowMs;
  final bool examAvailable;
  final VoidCallback onRestart;
  final VoidCallback onOpenExam;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    final reviews = state.reviews;
    final acquired = state.acquired;
    final accuracy = reviews > 0
        ? '${(100 * state.passed / reviews).round()}%'
        : '–';
    final headline = reviews > 0
        ? 'Nicely charged.'
        : acquired > 0
        ? 'New cards planted.'
        : 'Nothing due.';
    final nextDue = reviews == 0 && acquired == 0
        ? _nextDueNote(state.nextDueMs)
        : null;
    final noteText =
        nextDue ??
        (!state.canRestart ? 'Nothing due right now, come back later.' : null);
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
              letterSpacing: 2.2,
            ),
          ),
          const SizedBox(height: 14),
          Text(
            headline,
            style: TextStyle(
              fontSize: 26,
              fontWeight: FontWeight.w600,
              color: theme.colorScheme.onSurface,
            ),
          ),
          const SizedBox(height: 18),
          if (acquired > 0)
            _summaryRow(context, 'introduced', '$acquired', tokens),
          _summaryRow(context, 'reviewed', '$reviews', tokens),
          if (reviews > 0) ...[
            _summaryRow(
              context,
              'passed / failed',
              '${state.passed} / ${state.failed}',
              tokens,
            ),
            _summaryRow(context, 'accuracy', accuracy, tokens),
          ],
          if (noteText != null) ...[
            const SizedBox(height: 18),
            Container(
              width: double.infinity,
              padding: const EdgeInsets.symmetric(horizontal: 15, vertical: 12),
              decoration: BoxDecoration(
                color: tokens.noteBorder.withValues(alpha: 0.12),
                border: Border.all(
                  color: tokens.noteBorder.withValues(alpha: 0.24),
                ),
                borderRadius: BorderRadius.circular(10),
              ),
              child: Text(
                noteText,
                style: TextStyle(color: tokens.noteInk, fontSize: 15),
              ),
            ),
          ],
          const SizedBox(height: 24),
          ReviewChip(
            label: 'New session',
            kind: state.canRestart
                ? ReviewChipKind.primary
                : ReviewChipKind.base,
            onTap: state.canRestart ? onRestart : null,
          ),
          if (examAvailable) ...[
            const SizedBox(height: 12),
            ReviewChip(
              label: 'Take the exam',
              kind: ReviewChipKind.quiet,
              onTap: onOpenExam,
            ),
          ],
        ],
      ),
    );
  }

  String? _nextDueNote(int? nextDueMs) {
    if (nextDueMs == null) return null;
    final delta = nextDueMs - nowMs;
    if (delta <= 0) return null;
    final minutes = (delta / 60000).round();
    if (minutes < 60) return 'Next due in ${minutes < 1 ? 1 : minutes} min.';
    final hours = (delta / 3600000).round();
    if (hours < 24) return 'Next due in $hours h.';
    final days = (delta / 86400000).round();
    return days <= 1 ? 'Next due tomorrow.' : 'Next due in $days days.';
  }

  Widget _summaryRow(
    BuildContext context,
    String label,
    String value,
    AlixTokens tokens,
  ) {
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
              color: Theme.of(context).colorScheme.onSurface,
            ),
          ),
        ],
      ),
    );
  }
}
