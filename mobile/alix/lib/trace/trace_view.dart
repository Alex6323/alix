import 'package:flutter/material.dart';

import 'package:alix_mobile/leave_guard.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/trace/trace_models.dart';
import 'package:alix_mobile/trace/trace_summary.dart';
import 'package:alix_mobile/trace/trace_widgets.dart';

const _mono = 'IBM Plex Mono';

class TraceSessionView extends StatelessWidget {
  const TraceSessionView({
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

  final TraceSessionStateModel state;
  final TextEditingController predictionController;
  final bool examAvailable;
  final int? cooldownMs;
  final Future<bool> Function(BuildContext context) confirmLeave;
  final VoidCallback onReveal;
  final ValueChanged<TraceSessionGrade> onGrade;
  final VoidCallback onOpenExam;
  final VoidCallback onRestart;

  @override
  Widget build(BuildContext context) {
    final tokens = Theme.of(context).alix;
    final done = state.phase == TraceSessionPhaseModel.done;
    return LeaveGuard(
      finished: done,
      confirm: () => confirmLeave(context),
      child: Scaffold(
        appBar: alixAppBar(
          context,
          title: const SizedBox.shrink(),
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
                TraceSessionSaveBanner(saveError: saveError),
              if (!done) TraceSessionDescriptionEyebrow(state: state),
              Expanded(
                child: done
                    ? TraceSessionSummaryView(
                        state: state,
                        examAvailable: examAvailable,
                        cooldownMs: cooldownMs,
                        onOpenExam: onOpenExam,
                        onRestart: onRestart,
                      )
                    : Column(
                        children: [
                          Expanded(
                            child: TraceSessionPhaseBody(
                              state: state,
                              predictionController: predictionController,
                            ),
                          ),
                          TraceSessionFooter(
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
