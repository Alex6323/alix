import 'package:flutter/material.dart';

import 'package:alix_mobile/leave_guard.dart';
import 'package:alix_mobile/review/crumb_strip.dart';
import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/review_summary.dart';
import 'package:alix_mobile/theme.dart';

const _mono = 'IBM Plex Mono';

class ReviewCantOpenView extends StatelessWidget {
  const ReviewCantOpenView({super.key, required this.message});

  final String message;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    return Scaffold(
      appBar: alixAppBar(context),
      body: Center(
        child: Padding(
          padding: const EdgeInsets.all(32),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                'CAN\'T OPEN THIS DECK',
                style: TextStyle(
                  fontFamily: _mono,
                  fontSize: 12,
                  letterSpacing: 2,
                  color: tokens.faint,
                ),
              ),
              const SizedBox(height: 16),
              Text(
                message,
                textAlign: TextAlign.center,
                style: theme.textTheme.bodyMedium?.copyWith(color: tokens.dim),
              ),
              const SizedBox(height: 24),
              FilledButton(
                onPressed: () => Navigator.of(context).maybePop(),
                child: const Text('Back'),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class ReviewView extends StatelessWidget {
  const ReviewView({
    super.key,
    required this.state,
    required this.crumb,
    required this.foreignWriter,
    required this.revealed,
    required this.revealedLines,
    required this.choice,
    required this.checkFeedback,
    required this.tickedKeypoints,
    required this.attemptOpen,
    required this.attemptController,
    required this.typedControllers,
    required this.serverLive,
    required this.tutorCard,
    required this.verdictGrade,
    required this.examAvailable,
    required this.nowMs,
    required this.confirmLeave,
    required this.onDismissForeignWriter,
    required this.onChoose,
    required this.onCheck,
    required this.onOpenAttempt,
    required this.onToggleKeypoint,
    required this.onReveal,
    required this.onRevealNextLine,
    required this.onAcquire,
    required this.onGrade,
    required this.onOpenTutor,
    required this.onRestart,
    required this.onOpenExam,
  });

  final ReviewStateModel state;
  final ReviewCrumbModel? crumb;
  final ReviewForeignWriterModel? foreignWriter;
  final bool revealed;
  final int revealedLines;
  final ReviewChoiceFeedbackModel? choice;
  final ReviewCheckFeedbackModel? checkFeedback;
  final Set<int> tickedKeypoints;
  final bool attemptOpen;
  final TextEditingController attemptController;
  final List<TextEditingController> typedControllers;
  final bool serverLive;
  final ReviewTutorCardModel? tutorCard;
  final ReviewGrade verdictGrade;
  final bool examAvailable;
  final int nowMs;
  final Future<bool> Function(BuildContext context) confirmLeave;
  final VoidCallback onDismissForeignWriter;
  final ValueChanged<int> onChoose;
  final ValueChanged<List<String>> onCheck;
  final VoidCallback onOpenAttempt;
  final ValueChanged<int> onToggleKeypoint;
  final VoidCallback onReveal;
  final VoidCallback onRevealNextLine;
  final VoidCallback onAcquire;
  final ValueChanged<ReviewGrade> onGrade;
  final ValueChanged<ReviewTutorCardModel> onOpenTutor;
  final VoidCallback onRestart;
  final VoidCallback onOpenExam;

  @override
  Widget build(BuildContext context) {
    final card = state.card;
    return LeaveGuard(
      finished: state.finished,
      confirm: () => confirmLeave(context),
      child: Scaffold(
        appBar: alixAppBar(
          context,
          actions: [
            if (!state.finished)
              Padding(
                padding: const EdgeInsets.only(right: 16),
                child: Center(
                  child: Text(
                    '${state.remaining} left',
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
              if (crumb == null || crumb!.regions.isEmpty)
                const SizedBox.shrink()
              else
                CrumbStrip(crumb: crumb!),
              if (foreignWriter case final writer?)
                _ForeignWriterBanner(
                  writer: writer,
                  onDismiss: onDismissForeignWriter,
                ),
              if (state.saveError case final saveError?)
                _SaveBanner(saveError: saveError),
              Expanded(
                child: card == null
                    ? ReviewSummaryView(
                        state: state,
                        nowMs: nowMs,
                        examAvailable: examAvailable,
                        onRestart: onRestart,
                        onOpenExam: onOpenExam,
                      )
                    : ReviewCardView(
                        state: state,
                        revealed: revealed,
                        revealedLines: revealedLines,
                        choice: choice,
                        checkFeedback: checkFeedback,
                        tickedKeypoints: tickedKeypoints,
                        attemptOpen: attemptOpen,
                        attemptController: attemptController,
                        typedControllers: typedControllers,
                        serverLive: serverLive,
                        tutorCard: tutorCard,
                        verdictGrade: verdictGrade,
                        onChoose: onChoose,
                        onCheck: onCheck,
                        onOpenAttempt: onOpenAttempt,
                        onToggleKeypoint: onToggleKeypoint,
                        onReveal: onReveal,
                        onRevealNextLine: onRevealNextLine,
                        onAcquire: onAcquire,
                        onGrade: onGrade,
                        onOpenTutor: onOpenTutor,
                      ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _SaveBanner extends StatelessWidget {
  const _SaveBanner({required this.saveError});

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

class _ForeignWriterBanner extends StatelessWidget {
  const _ForeignWriterBanner({required this.writer, required this.onDismiss});

  final ReviewForeignWriterModel writer;
  final VoidCallback onDismiss;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final minutes = (writer.ageMs / 60000).round();
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
              "Last written by '${writer.device}' $age ago. "
              'Review on one device at a time.',
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.colorScheme.onErrorContainer,
              ),
            ),
          ),
          IconButton(
            icon: const Icon(Icons.close, size: 18),
            onPressed: onDismiss,
          ),
        ],
      ),
    );
  }
}
