import 'package:flutter/material.dart';

import 'package:alix_mobile/leave_guard.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/walk/walk_models.dart';
import 'package:alix_mobile/walk/walk_summary.dart';
import 'package:alix_mobile/walk/walk_widgets.dart';

const _mono = 'IBM Plex Mono';

class WalkView extends StatelessWidget {
  const WalkView({
    super.key,
    required this.state,
    required this.predictionController,
    required this.examAvailable,
    required this.cooldownMs,
    required this.confirmLeave,
    required this.onReveal,
    required this.onGrade,
    required this.onOpenExam,
    required this.onRestart,
  });

  final WalkStateModel state;
  final TextEditingController predictionController;
  final bool examAvailable;
  final int? cooldownMs;
  final Future<bool> Function(BuildContext context) confirmLeave;
  final VoidCallback onReveal;
  final ValueChanged<WalkGrade> onGrade;
  final VoidCallback onOpenExam;
  final VoidCallback onRestart;

  @override
  Widget build(BuildContext context) {
    final tokens = Theme.of(context).alix;
    final done = state.phase == WalkPhaseModel.done;
    return LeaveGuard(
      finished: done,
      confirm: () => confirmLeave(context),
      child: Scaffold(
        appBar: alixAppBar(
          context,
          actions: [
            if (!done)
              Padding(
                padding: const EdgeInsets.only(right: 16),
                child: Center(
                  child: Text(
                    'checkpoint ${state.current} / ${state.total}',
                    style: TextStyle(
                      fontFamily: _mono,
                      fontSize: 13,
                      color: tokens.dim,
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
                WalkSaveBanner(saveError: saveError),
              if (!done) WalkDescriptionEyebrow(state: state),
              Expanded(
                child: done
                    ? WalkSummaryView(
                        state: state,
                        examAvailable: examAvailable,
                        cooldownMs: cooldownMs,
                        onOpenExam: onOpenExam,
                        onRestart: onRestart,
                      )
                    : Column(
                        children: [
                          Expanded(
                            child: WalkPhaseBody(
                              state: state,
                              predictionController: predictionController,
                            ),
                          ),
                          WalkFooter(
                            phase: state.phase,
                            onReveal: onReveal,
                            onGrade: onGrade,
                          ),
                        ],
                      ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
