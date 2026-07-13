import 'package:flutter/material.dart';

import 'package:alix_mobile/src/rust/api/review.dart';

/// The one M1 screen: renders the ReviewState the embedded core returns and
/// feeds grades back into it. All review logic lives in Rust; this widget
/// only shows the current position and forwards the learner's action.
class ReviewScreen extends StatefulWidget {
  const ReviewScreen({
    super.key,
    required this.deckPath,
    required this.storeDir,
  });

  final String deckPath;
  final String storeDir;

  @override
  State<ReviewScreen> createState() => _ReviewScreenState();
}

class _ReviewScreenState extends State<ReviewScreen> {
  late ReviewSession _session;
  late ReviewState _state;
  bool _unseen = false;
  bool _revealed = false;

  @override
  void initState() {
    super.initState();
    _open();
  }

  /// (Re)opens the session over the same deck and store; also the restart
  /// action on the done screen, picking up whatever has come due since.
  void _open() {
    _session = ReviewSession.open(
      deckPath: widget.deckPath,
      storeDir: widget.storeDir,
    );
    _state = _session.state();
    _unseen = _session.unseen();
    _revealed = false;
  }

  void _apply(ReviewState next) {
    setState(() {
      _state = next;
      _unseen = _session.unseen();
      _revealed = false;
    });
  }

  @override
  Widget build(BuildContext context) {
    final card = _state.card;
    return Scaffold(
      appBar: AppBar(
        title: const Text('alix'),
        actions: [
          if (!_state.finished)
            Padding(
              padding: const EdgeInsets.only(right: 16),
              child: Center(child: Text('${_state.remaining} left')),
            ),
        ],
      ),
      body: Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: card == null ? _done(context) : _card(context, card),
        ),
      ),
    );
  }

  Widget _done(BuildContext context) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Text('Done for now', style: Theme.of(context).textTheme.headlineSmall),
        const SizedBox(height: 8),
        const Text('Cards come back once they are due again.'),
        const SizedBox(height: 24),
        OutlinedButton(
          onPressed: () => setState(_open),
          child: const Text('Check again'),
        ),
      ],
    );
  }

  Widget _card(BuildContext context, CardView card) {
    final showBack = _unseen || _revealed;
    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        Expanded(
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Text(
                card.front,
                textAlign: TextAlign.center,
                style: Theme.of(context).textTheme.headlineSmall,
              ),
              if (showBack) ...[
                const SizedBox(height: 24),
                Text(
                  card.back.join('\n'),
                  textAlign: TextAlign.center,
                  style: Theme.of(context).textTheme.titleLarge?.copyWith(
                    color: Theme.of(context).colorScheme.primary,
                  ),
                ),
              ],
            ],
          ),
        ),
        if (_unseen)
          // A first exposure is acquired (attempt-first), not quizzed cold.
          FilledButton(
            onPressed: () => _apply(_session.acquire()),
            child: const Text('Seen'),
          )
        else if (!_revealed)
          FilledButton(
            onPressed: () => setState(() => _revealed = true),
            child: const Text('Reveal'),
          )
        else
          Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              for (final (label, grade) in [
                ('Fail', Grade.fail),
                ('Partial', Grade.partial),
                ('Pass', Grade.pass),
              ])
                Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 6),
                  child: FilledButton.tonal(
                    onPressed: () => _apply(_session.grade(grade: grade)),
                    child: Text(label),
                  ),
                ),
            ],
          ),
      ],
    );
  }
}
