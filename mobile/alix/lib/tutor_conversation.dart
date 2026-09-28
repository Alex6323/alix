// One card's tutor conversation, owned by the review screen so that closing
// the sheet loses nothing: the transcript, the job in flight, its poll timer
// and the note watermark live here, and a note or draft result applies when
// it arrives whether the sheet is open or not. The sheet is a view over this
// object. Data and callbacks only: this file never imports the generated
// bridge, so its tests run without a Rust dylib.
import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge.dart'
    show AnyhowException;

import 'package:alix_mobile/bridge/bridge_error.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/server_client.dart';

/// The exact wording for a 401 mid-conversation: the paired server is right
/// there but rejects this app's token (a restarted desktop mints a fresh
/// one). Every [ServerClient] call can throw [PairingExpired]; every call
/// site here catches it and reports exactly this line.
const pairingExpiredMessage =
    'Pairing expired. Pair again from Settings → Connected devices.';

/// Shown under the latest reply when the paired desktop answered from the
/// card alone (it could not match the card to a deck it serves).
const cardOnlyMessage =
    'Answered from the card alone: the paired desktop could not match this '
    'card.';

/// One settled exchange: the question, the raw answer (re-sent verbatim as
/// history), the answer's display units, and whether the desktop answered
/// from the card alone.
class TutorExchange {
  const TutorExchange({
    required this.q,
    required this.a,
    required this.units,
    required this.cardOnly,
  });

  final String q;
  final String a;
  final List<ReviewContentUnitModel> units;
  final bool cardOnly;
}

/// The phone's mirror of the desktop's one remote ask slot: the server
/// replaces a settled job on the next POST whether or not its client has
/// read the result, so only the conversation holding this slot may POST,
/// and it holds the slot until its call settles.
class TutorSlot extends ChangeNotifier {
  TutorConversation? _owner;
  TutorConversation? get owner => _owner;

  bool _take(TutorConversation conversation) {
    if (_owner != null && _owner != conversation) return false;
    _owner = conversation;
    return true;
  }

  void _release(TutorConversation conversation) {
    if (_owner != conversation) return;
    _owner = null;
    notifyListeners();
  }
}

class TutorConversation extends ChangeNotifier {
  TutorConversation({
    required this.card,
    required this.client,
    required this.slot,
    required this.mint,
    required this.onNote,
    required this.onMessage,
    this.pollInterval = const Duration(milliseconds: 400),
  }) {
    slot.addListener(_changed);
    _fetchBackendName();
  }

  /// The current card's authored fields, sent whole on every call (the
  /// server holds no session of its own for a remote turn).
  final TutorCardContext card;

  /// The paired desktop's AI backend, over `/api/remote/*`.
  final ServerClient client;

  /// Shared with every other conversation on the same desktop.
  final TutorSlot slot;

  /// Mints a drafted card from the edited front/back (a closure over the
  /// bridge session's `mintTutorCard`). Throws on a rejected mint (e.g. a
  /// duplicate); the thrown message is shown verbatim.
  final Future<String> Function(String front, List<String> back) mint;

  /// Applies the condensed note lines the desktop hands back to the deck (a
  /// closure over the bridge session's `applyCardNote`). Synchronous, unlike
  /// [mint]: `applyCardNote` does not fail the way a mint can.
  final void Function(List<String> notes) onNote;

  /// Every one-line message for the learner, as it is set on [message]; the
  /// owner surfaces it when the sheet is not there to show it inline.
  final void Function(String text) onMessage;

  /// How often to poll `GET /api/remote/ask` while a turn, a draft or a
  /// note is in flight. Tests shrink this well below the default.
  final Duration pollInterval;

  final List<TutorExchange> _transcript = [];
  List<TutorExchange> get transcript => List.unmodifiable(_transcript);

  String? _pendingQuestion;
  String? get pendingQuestion => _pendingQuestion;
  int? _pendingElapsed;
  int? get pendingElapsed => _pendingElapsed;

  /// The question a failed turn hands back to the composer, taken once.
  String? _restoredQuestion;
  String? takeRestoredQuestion() {
    final question = _restoredQuestion;
    _restoredQuestion = null;
    return question;
  }

  bool _draftPending = false;
  bool get draftPending => _draftPending;
  int? _draftElapsed;
  int? get draftElapsed => _draftElapsed;

  /// The drafted card awaiting the learner's edit, null when none.
  DraftCard? _draft;
  DraftCard? get draft => _draft;

  bool _notePending = false;
  bool get notePending => _notePending;
  int? _noteElapsed;
  int? get noteElapsed => _noteElapsed;

  /// The transcript index of the first exchange nothing was made from yet:
  /// a saved note or a draft consumes every exchange before it. Mirrors the
  /// desktop's `noted_through`.
  int _notedThrough = 0;
  int get notedThrough => _notedThrough;

  /// The desktop's context warning for the latest reply, null when none.
  String? _status;
  String? get status => _status;

  /// The latest one-line message for the learner (a saved note, a failed
  /// call, an expired pairing), cleared by the next action.
  String? _message;
  String? get message => _message;

  /// Fetched once, then cached; null reads as "the backend" in the working
  /// row (a plain refusal is not worth its own error UI).
  String? _backendName;
  String? get backendName => _backendName;

  Timer? _pollTimer;
  bool _disposed = false;

  @override
  void dispose() {
    _disposed = true;
    _pollTimer?.cancel();
    slot.removeListener(_changed);
    slot._release(this);
    super.dispose();
  }

  void _changed() {
    if (!_disposed) notifyListeners();
  }

  void _say(String text) {
    _message = text;
    _changed();
    onMessage(text);
  }

  /// A call of this conversation is in flight: ask, draft and note share
  /// this object's one poll timer.
  bool get busy => _pendingQuestion != null || _draftPending || _notePending;

  /// Another conversation's call holds the desktop's ask slot, so this one
  /// waits (the learner moved on while a note or draft was in flight).
  bool get slotBusy => slot.owner != null && slot.owner != this;

  bool get canSend => !busy && !slotBusy;

  /// Whether "Make this a note" and "Make this a card" may run: an answer
  /// exists that nothing was made from yet, and no call is in flight. The
  /// same gate the desktop serves as `AskDto.can_distill`.
  bool get canDistill => canSend && _transcript.length > _notedThrough;

  /// The latest reply was answered from the card alone.
  bool get latestCardOnly =>
      _transcript.isNotEmpty && _transcript.last.cardOnly;

  Future<void> _fetchBackendName() async {
    try {
      final name = await client.backendName();
      if (_disposed) return;
      _backendName = name;
      _changed();
    } on PairingExpired {
      if (_disposed) return;
      _say(pairingExpiredMessage);
    }
  }

  List<TutorTurn> _history({int from = 0}) => [
        for (final exchange in _transcript.skip(from))
          TutorTurn(q: exchange.q, a: exchange.a),
      ];

  /// One GET in flight at a time: the next poll is scheduled only after the
  /// awaited one, and a tick returns whether the call is still in flight.
  /// Overlapping GETs would each read the desktop's one terminal DTO and
  /// apply it twice.
  void _poll(Future<bool> Function() tick) {
    _pollTimer?.cancel();
    _pollTimer = Timer(pollInterval, () async {
      final inFlight = await tick();
      if (_disposed) return;
      if (inFlight) {
        _poll(tick);
      } else {
        slot._release(this);
      }
    });
  }

  /// A call ended before its poll started (a refused POST, an expired
  /// pairing): the slot goes back at once.
  void _settledWithoutPoll() => slot._release(this);

  // ── send ──────────────────────────────────────────────────────────────

  Future<void> send(String question) async {
    final text = question.trim();
    if (text.isEmpty || !canSend || !slot._take(this)) return;
    final history = _history();
    _pendingQuestion = text;
    _pendingElapsed = null;
    _message = null;
    _changed();

    bool ok;
    try {
      ok = await client.postAsk(card, history, text);
    } on PairingExpired {
      if (_disposed) return;
      _pendingQuestion = null;
      _settledWithoutPoll();
      _say(pairingExpiredMessage);
      return;
    }
    if (_disposed) return;
    if (!ok) {
      _pendingQuestion = null;
      _settledWithoutPoll();
      _say('The desktop did not answer.');
      return;
    }
    _poll(_pollAsk);
  }

  Future<bool> _pollAsk() async {
    RemoteAsk? dto;
    try {
      dto = await client.getAsk();
    } on PairingExpired {
      if (_disposed) return false;
      _pendingQuestion = null;
      _say(pairingExpiredMessage);
      return false;
    }
    if (_disposed) return false;
    if (dto == null) return true;
    if (dto.thinking) {
      _pendingElapsed = dto.elapsed;
      _changed();
      return true;
    }
    if (dto.error != null) {
      // Nothing is lost: the question goes back to the composer rather than
      // the transcript, since it never got a real answer.
      _restoredQuestion = _pendingQuestion;
      _pendingQuestion = null;
      _pendingElapsed = null;
      _say('The tutor call failed.');
      return false;
    }
    _transcript.add(TutorExchange(
      q: _pendingQuestion ?? '',
      a: dto.answer ?? '',
      units: dto.units,
      cardOnly: dto.cardOnly,
    ));
    _status = dto.status;
    _pendingQuestion = null;
    _pendingElapsed = null;
    _changed();
    return false;
  }

  // ── make a card ───────────────────────────────────────────────────────

  Future<void> makeCard() async {
    if (!canDistill || !slot._take(this)) return;
    final history = _history();
    _draftPending = true;
    _draftElapsed = null;
    _message = null;
    _changed();

    bool ok;
    try {
      ok = await client.postDraft(card, history);
    } on PairingExpired {
      if (_disposed) return;
      _draftPending = false;
      _settledWithoutPoll();
      _say(pairingExpiredMessage);
      return;
    }
    if (_disposed) return;
    if (!ok) {
      _draftPending = false;
      _settledWithoutPoll();
      _say('The desktop did not answer.');
      return;
    }
    _poll(_pollDraft);
  }

  Future<bool> _pollDraft() async {
    RemoteAsk? dto;
    try {
      dto = await client.getAsk();
    } on PairingExpired {
      if (_disposed) return false;
      _draftPending = false;
      _say(pairingExpiredMessage);
      return false;
    }
    if (_disposed) return false;
    if (dto == null) return true;
    if (dto.thinking) {
      _draftElapsed = dto.elapsed;
      _changed();
      return true;
    }
    final draft = dto.draft;
    if (dto.error != null || draft == null) {
      _draftPending = false;
      _say('The tutor call failed.');
      return false;
    }
    _draftPending = false;
    _draft = draft;
    // A draft consumes the exchanges it was made from, as on the desktop.
    _notedThrough = _transcript.length;
    _changed();
    return false;
  }

  /// Mints the edited draft; on a rejected mint the message is shown and the
  /// draft stays open for another try.
  Future<bool> confirmDraft(String front, List<String> back) async {
    try {
      await mint(front, back);
    } on AnyhowException catch (e) {
      if (_disposed) return false;
      _say(bridgeErrorText(e));
      return false;
    }
    if (_disposed) return true;
    _draft = null;
    _say('Card added.');
    return true;
  }

  void cancelDraft() {
    _draft = null;
    _changed();
  }

  // ── make a note ───────────────────────────────────────────────────────

  Future<void> makeNote() async {
    if (!canDistill || !slot._take(this)) return;
    // Only the exchanges nothing was made from yet, as the desktop condenses.
    final history = _history(from: _notedThrough);
    _notePending = true;
    _noteElapsed = null;
    _message = null;
    _changed();

    bool ok;
    try {
      ok = await client.postNote(card, history);
    } on PairingExpired {
      if (_disposed) return;
      _notePending = false;
      _settledWithoutPoll();
      _say(pairingExpiredMessage);
      return;
    }
    if (_disposed) return;
    if (!ok) {
      _notePending = false;
      _settledWithoutPoll();
      _say('The desktop did not answer.');
      return;
    }
    _poll(_pollNote);
  }

  Future<bool> _pollNote() async {
    RemoteAsk? dto;
    try {
      dto = await client.getAsk();
    } on PairingExpired {
      if (_disposed) return false;
      _notePending = false;
      _say(pairingExpiredMessage);
      return false;
    }
    if (_disposed) return false;
    if (dto == null) return true;
    if (dto.thinking) {
      _noteElapsed = dto.elapsed;
      _changed();
      return true;
    }
    if (dto.error != null) {
      _notePending = false;
      _say('The tutor call failed.');
      return false;
    }
    // Three states, per RemoteAsk.note's doc: null here means this settled
    // reply is not (yet) a note outcome, so keep polling.
    final notes = dto.note;
    if (notes == null) return true;
    _notePending = false;
    if (notes.isEmpty) {
      _say('nothing to save');
      return false;
    }
    _notedThrough = _transcript.length;
    onNote(notes);
    _say('note saved');
    return false;
  }
}
