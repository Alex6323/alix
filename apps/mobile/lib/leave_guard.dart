import 'package:flutter/material.dart';

/// Confirms abandoning an unfinished session. Returns true to leave, false to
/// stay. Shared by the review and walk screens so both deck kinds — fact and
/// trace — guard a stray back-swipe identically.
Future<bool> confirmLeaveSession(
  BuildContext context, {
  required String title,
  required String body,
  required String stayLabel,
}) async {
  final leave = await showDialog<bool>(
    context: context,
    builder: (dialogContext) => AlertDialog(
      title: Text(title),
      content: Text(body),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(dialogContext).pop(false),
          child: Text(stayLabel),
        ),
        TextButton(
          onPressed: () => Navigator.of(dialogContext).pop(true),
          child: const Text('Leave'),
        ),
      ],
    ),
  );
  return leave ?? false;
}

/// Wraps a session screen so a back gesture or the AppBar back is intercepted
/// while the session is unfinished, asking [confirm] before popping; a
/// `finished` screen pops immediately. Both session screens (review, walk) use
/// this one widget, so the leave guard can't silently go missing on one deck
/// kind the way it did before.
class LeaveGuard extends StatelessWidget {
  const LeaveGuard({
    super.key,
    required this.finished,
    required this.confirm,
    required this.child,
  });

  /// Nothing left to do — leaving needs no confirmation.
  final bool finished;

  /// Ask the user; returns true to leave, false to stay.
  final Future<bool> Function() confirm;

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return PopScope(
      canPop: finished,
      onPopInvokedWithResult: (didPop, _) async {
        if (didPop) return;
        final navigator = Navigator.of(context);
        if (await confirm() && navigator.mounted) navigator.pop();
      },
      child: child,
    );
  }
}
