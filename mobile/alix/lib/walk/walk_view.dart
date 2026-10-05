import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';

import 'package:alix_mobile/review/review_card.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/sketch.dart';
import 'package:alix_mobile/review/unit_widgets.dart' show monoFontFamily;
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/walk/walk_models.dart';

class WalkView extends StatelessWidget {
  const WalkView({
    super.key,
    required this.state,
    required this.choice,
    required this.multiChoice,
    required this.multiSelected,
    required this.sketch,
    required this.onSketchBegin,
    required this.onSketchExtend,
    required this.onSketchEnd,
    required this.onSketchTool,
    required this.onSketchUndo,
    required this.onSketchClear,
    required this.attemptController,
    required this.typedControllers,
    required this.serverLive,
    required this.tutorCard,
    required this.onChoose,
    required this.onToggleChoice,
    required this.onSubmitChoices,
    required this.onReveal,
    required this.onNext,
    required this.onNextWalk,
    required this.onOpenTutor,
    required this.onOpenSection,
  });

  final WalkStateModel state;
  final ReviewChoiceFeedbackModel? choice;
  final ReviewMultiChoiceFeedbackModel? multiChoice;
  final Set<int> multiSelected;
  final Sketch sketch;
  final void Function(Offset point, PointerDeviceKind kind) onSketchBegin;
  final ValueChanged<Offset> onSketchExtend;
  final VoidCallback onSketchEnd;
  final ValueChanged<SketchTool> onSketchTool;
  final VoidCallback onSketchUndo;
  final VoidCallback onSketchClear;
  final TextEditingController attemptController;
  final List<TextEditingController> typedControllers;
  final bool serverLive;
  final ReviewTutorCardModel? tutorCard;
  final ValueChanged<int> onChoose;
  final ValueChanged<int> onToggleChoice;
  final VoidCallback onSubmitChoices;
  final VoidCallback onReveal;
  final VoidCallback onNext;
  final VoidCallback onNextWalk;
  final ValueChanged<ReviewTutorCardModel> onOpenTutor;
  final ValueChanged<ReviewCardModel> onOpenSection;

  @override
  Widget build(BuildContext context) {
    final view = state.view;
    final card = view.card;
    final hasChoices = view.choices?.isNotEmpty ?? false;
    return Scaffold(
      appBar: alixAppBar(
        context,
        title: Padding(
          padding: const EdgeInsets.only(right: 12),
          child: Text(
            state.label,
            key: const ValueKey('walk-title'),
            maxLines: 1,
            softWrap: false,
            overflow: TextOverflow.ellipsis,
          ),
        ),
        actions: [
          if (!state.done)
            Padding(
              padding: const EdgeInsets.only(right: 16),
              child: Center(
                child: Text(
                  '${state.left} left',
                  style: TextStyle(
                    fontFamily: monoFontFamily,
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
            if (state.saveError case final saveError?)
              _SaveBanner(saveError: saveError),
            Expanded(child: _body(view, card, hasChoices)),
          ],
        ),
      ),
    );
  }

  Widget _body(ReviewStateModel view, ReviewCardModel? card, bool hasChoices) {
    return card == null
        ? WalkSummaryView(total: state.total, onNextWalk: onNextWalk)
        : ReviewCardView(
            state: view,
            revealed: state.phase == WalkPhase.answer && !hasChoices,
            revealedLines: 0,
            choice: choice,
            multiChoice: multiChoice,
            multiSelected: multiSelected,
            checkFeedback: null,
            typelineChecked: const [],
            tickedKeypoints: const {},
            sketch: sketch,
            onSketchBegin: onSketchBegin,
            onSketchExtend: onSketchExtend,
            onSketchEnd: onSketchEnd,
            onSketchTool: onSketchTool,
            onSketchUndo: onSketchUndo,
            onSketchClear: onSketchClear,
            attemptOpen: false,
            attemptController: attemptController,
            typedControllers: typedControllers,
            serverLive: serverLive,
            tutorCard: tutorCard,
            verdictGrade: ReviewGrade.pass,
            onChoose: onChoose,
            onToggleChoice: onToggleChoice,
            onSubmitChoices: onSubmitChoices,
            onCheck: (_) {},
            onOpenAttempt: () {},
            onToggleKeypoint: (_) {},
            onReveal: onReveal,
            onRevealNextLine: () {},
            onIntroduce: () {},
            onGrade: (_) {},
            onOpenTutor: onOpenTutor,
            onOpenSection: onOpenSection,
            onWalkNext: onNext,
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
          "Progress isn't being saved. The walk saves again with the next "
          'successful step.',
          style: theme.textTheme.bodySmall?.copyWith(
            color: theme.colorScheme.onErrorContainer,
          ),
        ),
      ),
    );
  }
}

class WalkSummaryView extends StatelessWidget {
  const WalkSummaryView({
    super.key,
    required this.total,
    required this.onNextWalk,
  });

  final int total;
  final VoidCallback onNextWalk;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    final walked = total > 0;
    return SingleChildScrollView(
      padding: const EdgeInsets.all(24),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 12),
          Text(
            walked ? 'WALK COMPLETE' : 'NOTHING TO WALK',
            style: TextStyle(
              fontFamily: monoFontFamily,
              color: tokens.bolt,
              fontSize: 11,
              letterSpacing: 2.2,
            ),
          ),
          const SizedBox(height: 14),
          Text(
            walked ? 'End of the deck.' : 'This deck has no cards.',
            style: TextStyle(
              fontSize: 26,
              fontWeight: FontWeight.w600,
              color: theme.colorScheme.onSurface,
            ),
          ),
          const SizedBox(height: 18),
          if (walked)
            Container(
              padding: const EdgeInsets.symmetric(vertical: 9),
              decoration: BoxDecoration(
                border: Border(bottom: BorderSide(color: tokens.line)),
              ),
              child: Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  Text('walked', style: TextStyle(color: tokens.dim)),
                  Text(
                    '$total',
                    style: TextStyle(
                      fontFamily: monoFontFamily,
                      fontWeight: FontWeight.w600,
                      color: theme.colorScheme.onSurface,
                    ),
                  ),
                ],
              ),
            ),
          const SizedBox(height: 24),
          ReviewChip(
            label: 'Next walk',
            kind: ReviewChipKind.primary,
            onTap: onNextWalk,
          ),
        ],
      ),
    );
  }
}
