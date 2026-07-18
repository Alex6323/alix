// The tutor sheet: a modal bottom sheet holding one phone-owned
// question/answer transcript against the paired desktop's AI backend, plus a
// tucked "Make a card" distillation flow. Data and callbacks only: this file
// never imports the generated bridge (`src/rust/*`), so its tests run
// without a Rust dylib. review_screen.dart owns the bridge session and hands
// this sheet a plain [TutorCardContext] and a mint closure over it.
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge.dart'
    show AnyhowException;

import 'package:alix_mobile/server_client.dart';

/// The exact wording for a 401 mid-conversation: the paired server is right
/// there but rejects this app's token (a restarted desktop mints a fresh
/// one). Every [ServerClient] call can throw [PairingExpired]; every call
/// site here catches it and shows exactly this line.
const _pairingExpiredMessage =
    'Pairing expired. Pair again from the deck list menu.';

/// One card's tutor conversation. Opened over the current review card;
/// closing it drops the transcript (nothing is persisted until "Make a
/// card" mints one).
class TutorSheet extends StatefulWidget {
  const TutorSheet({
    super.key,
    required this.card,
    required this.client,
    required this.mint,
    required this.onNote,
    this.pollInterval = const Duration(milliseconds: 400),
  });

  /// The current card's authored fields, sent whole on every call (the
  /// server holds no session of its own for a remote turn).
  final TutorCardContext card;

  /// The paired desktop's AI backend, over `/api/remote/*`.
  final ServerClient client;

  /// Mints a drafted card from the edited front/back (a closure over the
  /// bridge session's `mintTutorCard`); the sheet never calls the bridge
  /// itself. Throws on a rejected mint (e.g. a duplicate); the thrown
  /// message is shown verbatim.
  final Future<String> Function(String front, List<String> back) mint;

  /// Applies the condensed note lines the desktop hands back to the deck (a
  /// closure over the bridge session's `applyCardNote`); the sheet never
  /// calls the bridge itself. Synchronous, unlike [mint]: `applyCardNote`
  /// does not fail the way a mint can, so there is nothing to await or
  /// catch here.
  final void Function(List<String> notes) onNote;

  /// How often to poll `GET /api/remote/ask` while a turn or a draft is in
  /// flight. Tests shrink this well below the default.
  final Duration pollInterval;

  @override
  State<TutorSheet> createState() => _TutorSheetState();
}

class _TutorSheetState extends State<TutorSheet> {
  final _question = TextEditingController();
  final _draftFront = TextEditingController();
  final _draftBack = TextEditingController();

  /// Settled turns only; re-sent verbatim as `history` on every `postAsk`
  /// call (the server is stateless between turns).
  final List<(String q, String a)> _transcript = [];

  String? _pendingQuestion;
  int? _pendingElapsed;

  bool _draftPending = false;
  int? _draftElapsed;
  bool _editingDraft = false;

  bool _notePending = false;
  int? _noteElapsed;

  /// Fetched once per sheet open, then cached; null reads as "the backend"
  /// in the working row (a plain refusal is not worth its own error UI).
  String? _backendName;

  Timer? _pollTimer;

  @override
  void initState() {
    super.initState();
    _fetchBackendName();
  }

  @override
  void dispose() {
    _pollTimer?.cancel();
    _question.dispose();
    _draftFront.dispose();
    _draftBack.dispose();
    super.dispose();
  }

  Future<void> _fetchBackendName() async {
    try {
      final name = await widget.client.backendName();
      if (!mounted) return;
      setState(() => _backendName = name);
    } on PairingExpired {
      _pairingExpired();
    }
  }

  List<TutorTurn> _historyTurns() => [
        for (final (q, a) in _transcript) TutorTurn(q: q, a: a),
      ];

  // ── send ──────────────────────────────────────────────────────────────

  Future<void> _send() async {
    final question = _question.text.trim();
    if (question.isEmpty || _pendingQuestion != null || _draftPending || _notePending) return;
    final history = _historyTurns();
    setState(() {
      _pendingQuestion = question;
      _pendingElapsed = null;
    });
    _question.clear();

    bool ok;
    try {
      ok = await widget.client.postAsk(widget.card, history, question);
    } on PairingExpired {
      _pairingExpired();
      if (mounted) setState(() => _pendingQuestion = null);
      return;
    }
    if (!ok) {
      _snack('The desktop did not answer.');
      if (mounted) setState(() => _pendingQuestion = null);
      return;
    }
    // The sheet may have been dismissed while postAsk was in flight; a
    // timer started now would outlive dispose and never be cancelled.
    if (!mounted) return;
    _pollTimer?.cancel();
    _pollTimer = Timer.periodic(widget.pollInterval, (_) => _pollAsk());
  }

  Future<void> _pollAsk() async {
    RemoteAsk? dto;
    try {
      dto = await widget.client.getAsk();
    } on PairingExpired {
      _pollTimer?.cancel();
      _pairingExpired();
      if (mounted) setState(() => _pendingQuestion = null);
      return;
    }
    if (dto == null || !mounted) return;
    if (dto.thinking) {
      setState(() => _pendingElapsed = dto!.elapsed);
      return;
    }
    _pollTimer?.cancel();
    if (dto.error != null) {
      final question = _pendingQuestion;
      setState(() {
        // Nothing is lost: the question goes back in the input rather than
        // the transcript, since it never got a real answer.
        _question.text = question ?? '';
        _pendingQuestion = null;
        _pendingElapsed = null;
      });
      _snack('The tutor call failed.');
      return;
    }
    setState(() {
      _transcript.add((_pendingQuestion ?? '', dto!.answer ?? ''));
      _pendingQuestion = null;
      _pendingElapsed = null;
    });
  }

  // ── make a card ───────────────────────────────────────────────────────

  Future<void> _makeCard() async {
    if (_transcript.isEmpty) {
      _snack('Ask something first.');
      return;
    }
    final history = _historyTurns();
    setState(() {
      _draftPending = true;
      _draftElapsed = null;
    });

    bool ok;
    try {
      ok = await widget.client.postDraft(widget.card, history);
    } on PairingExpired {
      _pairingExpired();
      if (mounted) setState(() => _draftPending = false);
      return;
    }
    if (!ok) {
      _snack('The desktop did not answer.');
      if (mounted) setState(() => _draftPending = false);
      return;
    }
    // Same guard as _send: no timer may start once the sheet is gone.
    if (!mounted) return;
    _pollTimer?.cancel();
    _pollTimer = Timer.periodic(widget.pollInterval, (_) => _pollDraft());
  }

  Future<void> _pollDraft() async {
    RemoteAsk? dto;
    try {
      dto = await widget.client.getAsk();
    } on PairingExpired {
      _pollTimer?.cancel();
      _pairingExpired();
      if (mounted) setState(() => _draftPending = false);
      return;
    }
    if (dto == null || !mounted) return;
    if (dto.thinking) {
      setState(() => _draftElapsed = dto!.elapsed);
      return;
    }
    _pollTimer?.cancel();
    final draft = dto.draft;
    if (dto.error != null || draft == null) {
      setState(() => _draftPending = false);
      _snack('The tutor call failed.');
      return;
    }
    setState(() {
      _draftPending = false;
      _editingDraft = true;
      _draftFront.text = draft.front;
      _draftBack.text = draft.back.join('\n');
    });
  }

  Future<void> _confirmDraft() async {
    final front = _draftFront.text.trim();
    final back = _draftBack.text
        .split('\n')
        .map((line) => line.trim())
        .where((line) => line.isNotEmpty)
        .toList();
    try {
      await widget.mint(front, back);
    } on AnyhowException catch (e) {
      _snack(e.message);
      return;
    }
    if (!mounted) return;
    setState(() {
      _editingDraft = false;
      _draftFront.clear();
      _draftBack.clear();
    });
    _snack('Card added.');
  }

  void _cancelDraft() {
    setState(() {
      _editingDraft = false;
      _draftFront.clear();
      _draftBack.clear();
    });
  }

  // ── make a note ───────────────────────────────────────────────────────

  Future<void> _makeNote() async {
    if (_transcript.isEmpty) {
      _snack('Ask something first.');
      return;
    }
    final history = _historyTurns();
    setState(() {
      _notePending = true;
      _noteElapsed = null;
    });

    bool ok;
    try {
      ok = await widget.client.postNote(widget.card, history);
    } on PairingExpired {
      _pairingExpired();
      if (mounted) setState(() => _notePending = false);
      return;
    }
    if (!ok) {
      _snack('The desktop did not answer.');
      if (mounted) setState(() => _notePending = false);
      return;
    }
    // Same guard as _send/_makeCard: no timer may start once the sheet is
    // gone.
    if (!mounted) return;
    _pollTimer?.cancel();
    _pollTimer = Timer.periodic(widget.pollInterval, (_) => _pollNote());
  }

  Future<void> _pollNote() async {
    RemoteAsk? dto;
    try {
      dto = await widget.client.getAsk();
    } on PairingExpired {
      _pollTimer?.cancel();
      _pairingExpired();
      if (mounted) setState(() => _notePending = false);
      return;
    }
    if (dto == null || !mounted) return;
    if (dto.thinking) {
      setState(() => _noteElapsed = dto!.elapsed);
      return;
    }
    if (dto.error != null) {
      _pollTimer?.cancel();
      setState(() => _notePending = false);
      _snack('The tutor call failed.');
      return;
    }
    // Three states, per RemoteAsk.note's doc: null here means this settled
    // reply is not (yet) a note outcome, so keep polling; the timer is left
    // running and this tick is a no-op.
    final notes = dto.note;
    if (notes == null) return;
    _pollTimer?.cancel();
    setState(() => _notePending = false);
    if (notes.isEmpty) {
      _snack('nothing to save');
      return;
    }
    widget.onNote(notes);
    _snack('note saved');
  }

  // ── shared ────────────────────────────────────────────────────────────

  void _pairingExpired() {
    if (!mounted) return;
    ScaffoldMessenger.of(context)
        .showSnackBar(const SnackBar(content: Text(_pairingExpiredMessage)));
  }

  void _snack(String text) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(text)));
  }

  String _workingLabel(int? elapsed) {
    final who = _backendName ?? 'The backend';
    final suffix = elapsed != null ? ' ${elapsed}s' : '';
    return '$who is working…$suffix';
  }

  // ── build ─────────────────────────────────────────────────────────────

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return SafeArea(
      child: Padding(
        padding: EdgeInsets.fromLTRB(
            20, 20, 20, 20 + MediaQuery.of(context).viewInsets.bottom),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              widget.card.front,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: theme.textTheme.titleMedium,
            ),
            const SizedBox(height: 12),
            Flexible(
              child: SingleChildScrollView(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    for (final (q, a) in _transcript) _turn(theme, q, a),
                    if (_pendingQuestion != null)
                      _working(theme, _workingLabel(_pendingElapsed)),
                  ],
                ),
              ),
            ),
            const SizedBox(height: 12),
            if (_editingDraft) _draftEditor(theme) else _composer(theme),
          ],
        ),
      ),
    );
  }

  Widget _turn(ThemeData theme, String q, String a) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(q, style: theme.textTheme.bodyMedium?.copyWith(
              color: theme.colorScheme.onSurfaceVariant)),
          const SizedBox(height: 4),
          Text(a, style: theme.textTheme.bodyMedium),
        ],
      ),
    );
  }

  Widget _working(ThemeData theme, String label) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 8),
      child: Text(
        label,
        style: theme.textTheme.bodySmall
            ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
      ),
    );
  }

  Widget _composer(ThemeData theme) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(
          crossAxisAlignment: CrossAxisAlignment.end,
          children: [
            Expanded(
              child: TextField(
                key: const ValueKey('tutor-question-field'),
                controller: _question,
                minLines: 1,
                maxLines: 3,
                decoration: const InputDecoration(hintText: 'Ask about this card'),
              ),
            ),
            const SizedBox(width: 8),
            IconButton(
              key: const ValueKey('tutor-send-button'),
              icon: const Icon(Icons.send),
              // Disabled during a draft or a note too: ask, draft, and note
              // share the one poll timer, so a send now would orphan
              // whichever row is in flight.
              onPressed: _pendingQuestion == null && !_draftPending && !_notePending
                  ? _send
                  : null,
            ),
          ],
        ),
        Align(
          alignment: Alignment.centerRight,
          child: _draftPending
              ? Padding(
                  padding: const EdgeInsets.only(top: 4),
                  child: Text(
                    _workingLabel(_draftElapsed),
                    style: theme.textTheme.bodySmall
                        ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
                  ),
                )
              : _notePending
                  ? Padding(
                      padding: const EdgeInsets.only(top: 4),
                      child: Text(
                        _workingLabel(_noteElapsed),
                        style: theme.textTheme.bodySmall
                            ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
                      ),
                    )
                  : Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        TextButton(
                          key: const ValueKey('tutor-make-note-button'),
                          onPressed: _pendingQuestion == null ? _makeNote : null,
                          child: const Text('Make a note'),
                        ),
                        TextButton(
                          key: const ValueKey('tutor-make-card-button'),
                          onPressed:
                              _pendingQuestion == null ? _makeCard : null,
                          child: const Text('Make a card'),
                        ),
                      ],
                    ),
        ),
      ],
    );
  }

  Widget _draftEditor(ThemeData theme) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text('New card', style: theme.textTheme.titleSmall),
        const SizedBox(height: 8),
        TextField(
          key: const ValueKey('tutor-draft-front-field'),
          controller: _draftFront,
          decoration: const InputDecoration(labelText: 'Front'),
        ),
        const SizedBox(height: 8),
        TextField(
          key: const ValueKey('tutor-draft-back-field'),
          controller: _draftBack,
          minLines: 1,
          maxLines: 4,
          decoration: const InputDecoration(labelText: 'Back'),
        ),
        const SizedBox(height: 12),
        Row(
          mainAxisAlignment: MainAxisAlignment.end,
          children: [
            TextButton(
              key: const ValueKey('tutor-draft-cancel-button'),
              onPressed: _cancelDraft,
              child: const Text('Cancel'),
            ),
            const SizedBox(width: 8),
            FilledButton(
              key: const ValueKey('tutor-draft-confirm-button'),
              onPressed: _confirmDraft,
              child: const Text('Add'),
            ),
          ],
        ),
      ],
    );
  }
}
