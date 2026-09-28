// The tutor sheet: a modal bottom sheet showing one card's tutor
// conversation and its composer. The conversation itself (transcript, job in
// flight, note watermark) belongs to the review screen's [TutorConversation];
// this sheet is a view over it, so dismissing the sheet loses nothing and a
// result that lands while it is closed is there when it reopens. Data and
// callbacks only: this file never imports the generated bridge, so its tests
// run without a Rust dylib.
import 'dart:async';

import 'package:flutter/material.dart';

import 'package:alix_mobile/review/unit_widgets.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/tutor_conversation.dart';

class TutorSheet extends StatefulWidget {
  const TutorSheet({super.key, required this.conversation});

  final TutorConversation conversation;

  @override
  State<TutorSheet> createState() => _TutorSheetState();
}

class _TutorSheetState extends State<TutorSheet> {
  final _question = TextEditingController();
  final _draftFront = TextEditingController();
  final _draftBack = TextEditingController();

  /// The draft the editor fields were last filled from, so a draft that
  /// arrived while the sheet was closed fills them once on open and the
  /// learner's edits survive later rebuilds.
  DraftCard? _filledDraft;

  TutorConversation get _conversation => widget.conversation;

  @override
  void initState() {
    super.initState();
    _conversation.addListener(_onConversation);
    _syncFields();
  }

  @override
  void dispose() {
    _conversation.removeListener(_onConversation);
    _question.dispose();
    _draftFront.dispose();
    _draftBack.dispose();
    super.dispose();
  }

  void _onConversation() {
    _syncFields();
    if (mounted) setState(() {});
  }

  /// Pulls what the conversation holds for the composer and the draft
  /// editor: a question a failed turn handed back, a draft not yet edited.
  void _syncFields() {
    final restored = _conversation.takeRestoredQuestion();
    if (restored != null) _question.text = restored;
    final draft = _conversation.draft;
    if (draft != null && !identical(draft, _filledDraft)) {
      _filledDraft = draft;
      _draftFront.text = draft.front;
      _draftBack.text = draft.back.join('\n');
    }
  }

  void _send() {
    final question = _question.text.trim();
    if (question.isEmpty || !_conversation.canSend) return;
    _question.clear();
    unawaited(_conversation.send(question));
  }

  Future<void> _confirmDraft() async {
    final front = _draftFront.text.trim();
    final back = _draftBack.text
        .split('\n')
        .map((line) => line.trim())
        .where((line) => line.isNotEmpty)
        .toList();
    final added = await _conversation.confirmDraft(front, back);
    if (added && mounted) {
      _filledDraft = null;
      _draftFront.clear();
      _draftBack.clear();
    }
  }

  void _cancelDraft() {
    _filledDraft = null;
    _draftFront.clear();
    _draftBack.clear();
    _conversation.cancelDraft();
  }

  String _workingLabel(int? elapsed) {
    final who = _conversation.backendName ?? 'The backend';
    final suffix = elapsed != null ? ' ${elapsed}s' : '';
    return '$who is working…$suffix';
  }

  /// The desktop's one ask slot is held by the card the learner left.
  String _previousCardLabel() {
    final who = _conversation.backendName ?? 'The backend';
    return '$who is still working on the previous card…';
  }

  // ── build ─────────────────────────────────────────────────────────────

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = theme.extension<AlixTokens>()!;
    final conversation = _conversation;
    final transcript = conversation.transcript;
    return SafeArea(
      child: Padding(
        padding: EdgeInsets.fromLTRB(
            20, 20, 20, 20 + MediaQuery.of(context).viewInsets.bottom),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              conversation.card.front,
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
                    for (final (index, exchange) in transcript.indexed)
                      _turn(
                        theme,
                        tokens,
                        exchange,
                        cardOnly: index == transcript.length - 1 &&
                            exchange.cardOnly,
                      ),
                    if (conversation.status case final status?)
                      _dimLine(theme, status),
                    if (conversation.pendingQuestion != null)
                      _dimLine(theme, _workingLabel(conversation.pendingElapsed)),
                    if (conversation.message case final message?)
                      _dimLine(theme, message, key: const ValueKey('tutor-message')),
                  ],
                ),
              ),
            ),
            const SizedBox(height: 12),
            if (conversation.draft != null)
              _draftEditor(theme)
            else
              _composer(theme, conversation),
          ],
        ),
      ),
    );
  }

  Widget _turn(
    ThemeData theme,
    AlixTokens tokens,
    TutorExchange exchange, {
    required bool cardOnly,
  }) {
    final style = theme.textTheme.bodyMedium ?? const TextStyle();
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(exchange.q,
              style: style.copyWith(color: theme.colorScheme.onSurfaceVariant)),
          const SizedBox(height: 4),
          if (exchange.units.isEmpty)
            Text(exchange.a, style: style)
          else
            for (final unit in exchange.units)
              Padding(
                padding: const EdgeInsets.only(bottom: 4),
                child: unitWidget(unit, tokens, style, TextAlign.start),
              ),
          if (cardOnly)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Text(
                cardOnlyMessage,
                key: const ValueKey('tutor-card-only-line'),
                style: theme.textTheme.bodySmall
                    ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
              ),
            ),
        ],
      ),
    );
  }

  Widget _dimLine(ThemeData theme, String text, {Key? key}) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 8),
      child: Text(
        text,
        key: key,
        style: theme.textTheme.bodySmall
            ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
      ),
    );
  }

  Widget _composer(ThemeData theme, TutorConversation conversation) {
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
              onPressed: conversation.canSend ? _send : null,
            ),
          ],
        ),
        Align(
          alignment: Alignment.centerRight,
          child: conversation.draftPending ||
                  conversation.notePending ||
                  conversation.slotBusy
              ? Padding(
                  padding: const EdgeInsets.only(top: 4),
                  child: Text(
                    conversation.slotBusy
                        ? _previousCardLabel()
                        : _workingLabel(conversation.draftPending
                            ? conversation.draftElapsed
                            : conversation.noteElapsed),
                    style: theme.textTheme.bodySmall
                        ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
                  ),
                )
              : Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    TextButton(
                      key: const ValueKey('tutor-make-note-button'),
                      onPressed: conversation.canDistill
                          ? () => unawaited(conversation.makeNote())
                          : null,
                      // Matches the web's "Make this a note" chip wording.
                      child: const Text('Make this a note'),
                    ),
                    TextButton(
                      key: const ValueKey('tutor-make-card-button'),
                      onPressed: conversation.canDistill
                          ? () => unawaited(conversation.makeCard())
                          : null,
                      // Matches the web's "Make this a card" chip wording.
                      child: const Text('Make this a card'),
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
