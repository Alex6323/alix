// The one shared `ServerClient` test double (T8.4): every widget test that
// needs a fake desktop imports this instead of hand-rolling its own. Purely
// data-configured, as a fake must be: canned replies (queued per call, the
// last one repeating once exhausted), spy captures for the calls a test
// wants to assert on, and per-method PairingExpired/gate levers. It computes
// nothing production-shaped, it only plays back what a test told it to.
import 'dart:async';

import 'package:alix_mobile/server_client.dart';

/// A fake [ServerClient]: no network, no Rust dylib. Every constructor
/// parameter defaults to whatever the previous ~5 per-file duplicates
/// defaulted to, so a call site that only sets the fields it cares about
/// keeps its old behavior.
class FakeServerClient implements ServerClient {
  FakeServerClient({
    this.versionReply,
    this.rootIdReply = 'root-test0000000000000000000000',
    this.profileReply,
    this.expireOnVersion = false,
    this.versionGate,
    this.backendReply = 'Claude',
    List<bool>? postAskReplies,
    this.expireOnPostAsk = false,
    this.postAskGate,
    List<RemoteAsk>? getAskReplies,
    Map<int, Completer<RemoteAsk?>>? getAskGates,
    List<bool>? postDraftReplies,
    List<bool>? postNoteReplies,
    List<AddCardReply>? postCardReplies,
    this.expireOnPostCard = false,
    this.postCardGate,
    this.examStartReply = true,
    this.examStartSitting,
    this.examStartSittings,
    this.expireOnExamStart = false,
    List<RemoteExam>? examGetReplies,
    this.expireOnExamGet = false,
    this.expireOnExamGetCall,
    Map<int, Completer<RemoteExam?>>? examGetGates,
    this.nullFirstExamGetCalls = 0,
    this.noSittingBeforeStart = false,
    this.idleBeforeStart = false,
    this.examCloseGate,
    this.examCloseReply = true,
    this.expireOnExamClose = false,
    this.refuseStartWhileGenerating = false,
    this.failExamClose = false,
    this.examCarriedReply,
    this.examCarriedReplies,
    List<GradingCall>? gradingReplies,
    this.expireOnGrading = false,
    this.gradingCloseReply = true,
    this.gradingCloseGate,
    this.expireOnGradingClose = false,
    Map<int, Completer<GradingCall>>? gradingGates,
    this.callLog,
    this.generateStartReply = true,
    this.expireOnGenerateStart = false,
    List<RemoteGenerate>? generateGetReplies,
    this.expireOnGenerateGet = false,
  }) : postAskReplies = postAskReplies ?? const [true],
       getAskReplies = getAskReplies ?? const [],
       getAskGates = getAskGates ?? const {},
       postDraftReplies = postDraftReplies ?? const [true],
       postNoteReplies = postNoteReplies ?? const [true],
       postCardReplies = postCardReplies ?? const [],
       examGetReplies = examGetReplies ?? const [],
       examGetGates = examGetGates ?? const {},
       gradingReplies = gradingReplies ?? const [],
       gradingGates = gradingGates ?? const {},
       generateGetReplies = generateGetReplies ?? const [];

  // ── probe (version / backendName) ────────────────────────────────────
  final String? versionReply;

  /// `VersionDto.root_id` on a non-null [versionReply]; pass null to
  /// exercise the pairing sheet's "too old for sync" refusal.
  final String? rootIdReply;

  /// `VersionDto.profile`: the desktop's launch profile, null when it was
  /// started without one.
  final String? profileReply;
  final bool expireOnVersion;
  final Completer<void>? versionGate;
  final String? backendReply;

  // ── tutor (postAsk / getAsk / postDraft / postNote) ────────────────────
  final List<bool> postAskReplies;
  final bool expireOnPostAsk;

  /// When set, `postAsk` parks on this until the test completes it: the
  /// "request still in flight while the sheet is dismissed" lever.
  final Completer<bool>? postAskGate;
  final List<RemoteAsk> getAskReplies;

  /// Parks a specific (0-based) `getAsk()` call on a completer instead of
  /// answering from [getAskReplies]: the lever for a poll still in flight
  /// when the next tick would fire, or when another conversation acts.
  final Map<int, Completer<RemoteAsk?>> getAskGates;
  final List<bool> postDraftReplies;
  final List<bool> postNoteReplies;

  /// Replies to `postCard`, one per call, the last repeating; empty reads
  /// as no answer.
  final List<AddCardReply> postCardReplies;
  final bool expireOnPostCard;
  final Completer<AddCardReply>? postCardGate;

  // ── exam (the generation desk and the grading desk) ─────────────────────
  final bool examStartReply;

  /// The sitting id an accepted `examStart` names.
  final String? examStartSitting;

  /// The sitting each accepted `examStart` names, one per call, the last
  /// repeating; when set they replace [examStartSitting].
  final List<String?>? examStartSittings;
  final bool expireOnExamStart;
  final List<RemoteExam> examGetReplies;
  final bool expireOnExamGet;

  /// The one (0-based, counted) `examGet()` call that throws
  /// [PairingExpired].
  final int? expireOnExamGetCall;

  /// Parks a specific (0-based) `examGet()` call on a completer instead of
  /// answering immediately: the lever for putting two polls in flight at
  /// once (a periodic tick firing again before the prior call's own
  /// cancellation lands).
  final Map<int, Completer<RemoteExam?>> examGetGates;

  /// How many leading `examGet()` calls return null regardless of
  /// [examGetReplies], simulating a slow sitting start before the real
  /// replies begin.
  final int nullFirstExamGetCalls;

  /// `examGet()` answers null, without consuming a reply, until `examStart`
  /// has been called: the desktop holds no sitting the phone did not start.
  final bool noSittingBeforeStart;

  /// `examGet()` answers an `idle` desk, without consuming a reply, until
  /// `examStart` has been called.
  final bool idleBeforeStart;
  final Completer<void>? examCloseGate;
  final bool examCloseReply;

  /// `examClose` throws [PairingExpired] after recording its sitting.
  final bool expireOnExamClose;

  /// `examStart` answers no sitting once an earlier start was answered with
  /// one, as a desk does while that sitting is being generated.
  final bool refuseStartWhileGenerating;
  final bool failExamClose;

  final String? examCarriedReply;

  /// Replies to `examCarried`, one per call, the last repeating; when set
  /// they replace [examCarriedReply].
  final List<String?>? examCarriedReplies;

  /// Replies to `gradingSubmit`, `gradingRemediate` and `gradingGet`, one per
  /// call across the three, the last repeating; empty reads as no answer.
  final List<GradingCall> gradingReplies;
  final bool expireOnGrading;
  final bool gradingCloseReply;

  /// When set, `gradingClose` parks on this after recording its sitting.
  final Completer<void>? gradingCloseGate;

  /// `gradingClose` alone throws [PairingExpired], after its gate.
  final bool expireOnGradingClose;

  /// Parks a specific (0-based) grading call, counted across `gradingSubmit`,
  /// `gradingRemediate` and `gradingGet`, on a completer.
  final Map<int, Completer<GradingCall>> gradingGates;

  /// When set, every grading desk call appends its name here as it is made,
  /// so a test can read the order of these calls beside its own.
  final List<String>? callLog;

  // ── generate (generateStart / generateGet) ──────────────────────────────
  final bool generateStartReply;
  final bool expireOnGenerateStart;
  final List<RemoteGenerate> generateGetReplies;
  final bool expireOnGenerateGet;

  // ── spies ────────────────────────────────────────────────────────────
  final List<List<TutorTurn>> postAskHistories = [];
  final List<List<TutorTurn>> postDraftHistories = [];
  final List<List<TutorTurn>> postNoteHistories = [];
  final List<PostCardCall> postCardCalls = [];

  /// The cards argument of the most recent `examStart` call.
  List<String>? startedCards;

  /// The deck argument of the most recent `examStart` call.
  String? startedDeck;

  Map<String, dynamic>? startedIdeas;

  int examStartCalls = 0;

  bool generateStartCalled = false;
  String? generateStartedUrl;
  String? generateStartedGuidance;
  final List<String> gradingRequests = [];
  final List<String> remediateRequests = [];
  final List<String> carriedSittings = [];
  final List<String> examClosedSittings = [];
  final List<String> gradingClosedSittings = [];
  int examCloseCalls = 0;
  int generateCloseCalls = 0;
  bool closed = false;

  int _askCall = 0;
  int _pollCall = 0;
  int _draftCall = 0;
  int _noteCall = 0;
  int _cardCall = 0;
  int _examGetCall = 0;
  int get examGetCalls => _examGetCall;

  /// Every `examGet()` call, including those answered before the counted
  /// replies (a replacement, an idle desk before any start).
  int examGetAnswered = 0;
  int _gradingCall = 0;
  int _gradingCallsMade = 0;
  int _generateGetCall = 0;
  RemoteExam? _replacementExam;

  void replaceExam(RemoteExam exam) {
    _replacementExam = exam;
  }

  @override
  Future<ServerVersion?> version() async {
    if (versionGate != null) await versionGate!.future;
    if (expireOnVersion) throw const PairingExpired();
    final reply = versionReply;
    if (reply == null) return null;
    return ServerVersion(
      version: reply,
      rootId: rootIdReply,
      profile: profileReply,
    );
  }

  @override
  Future<String?> backendName() async => backendReply;

  @override
  Future<bool> postAsk(
    TutorCardContext card,
    List<TutorTurn> history,
    String question,
  ) async {
    postAskHistories.add(history);
    if (expireOnPostAsk) throw const PairingExpired();
    if (postAskGate != null) return postAskGate!.future;
    final reply = postAskReplies[_askCall.clamp(0, postAskReplies.length - 1)];
    _askCall++;
    return reply;
  }

  /// How many `getAsk()` calls have been made.
  int get getAskCalls => _pollCall;

  @override
  Future<RemoteAsk?> getAsk() async {
    final call = _pollCall++;
    final gate = getAskGates[call];
    if (gate != null) return gate.future;
    if (getAskReplies.isEmpty) return null;
    return getAskReplies[call.clamp(0, getAskReplies.length - 1)];
  }

  @override
  Future<bool> postDraft(TutorCardContext card, List<TutorTurn> history) async {
    postDraftHistories.add(history);
    final reply =
        postDraftReplies[_draftCall.clamp(0, postDraftReplies.length - 1)];
    _draftCall++;
    return reply;
  }

  @override
  Future<bool> postNote(TutorCardContext card, List<TutorTurn> history) async {
    postNoteHistories.add(history);
    final reply =
        postNoteReplies[_noteCall.clamp(0, postNoteReplies.length - 1)];
    _noteCall++;
    return reply;
  }

  @override
  Future<AddCardReply> postCard(
    String deckId,
    String cardId,
    String front,
    List<String> back,
  ) async {
    postCardCalls.add(PostCardCall(deckId: deckId, cardId: cardId, front: front, back: back));
    if (expireOnPostCard) throw const PairingExpired();
    if (postCardGate != null) return postCardGate!.future;
    if (postCardReplies.isEmpty) return const AddCardNoAnswer();
    final reply =
        postCardReplies[_cardCall.clamp(0, postCardReplies.length - 1)];
    _cardCall++;
    return reply;
  }

  @override
  Future<RemoteExam?> examStart(
    String deck, {
    List<String>? cards,
    Map<String, dynamic>? ideas,
  }) async {
    final call = examStartCalls++;
    startedDeck = deck;
    startedCards = cards;
    startedIdeas = ideas;
    if (expireOnExamStart) throw const PairingExpired();
    if (!examStartReply) return null;
    if (refuseStartWhileGenerating && call > 0) return null;
    final sittings = examStartSittings;
    return RemoteExam(
      sitting: sittings != null && sittings.isNotEmpty
          ? sittings[call.clamp(0, sittings.length - 1)]
          : examStartSitting,
      phase: 'generating',
      deck: deck,
      thinking: true,
    );
  }

  @override
  Future<RemoteExam?> examGet() async {
    examGetAnswered++;
    if (expireOnExamGet) throw const PairingExpired();
    if (_replacementExam case final replacement?) return replacement;
    if (noSittingBeforeStart && startedDeck == null) return null;
    if (idleBeforeStart && startedDeck == null) {
      return const RemoteExam(phase: 'idle', deck: '', thinking: false);
    }
    final call = _examGetCall;
    _examGetCall++;
    if (call == expireOnExamGetCall) throw const PairingExpired();
    final gate = examGetGates[call];
    if (gate != null) return gate.future;
    if (call < nullFirstExamGetCalls) return null;
    if (examGetReplies.isEmpty) return null;
    final repliesCall = call - nullFirstExamGetCalls;
    return examGetReplies[repliesCall.clamp(0, examGetReplies.length - 1)];
  }

  @override
  Future<String?> examCarried(String sitting) async {
    final call = carriedSittings.length;
    carriedSittings.add(sitting);
    if (expireOnExamGet) throw const PairingExpired();
    final replies = examCarriedReplies;
    if (replies != null && replies.isNotEmpty) {
      return replies[call.clamp(0, replies.length - 1)];
    }
    return examCarriedReply;
  }

  @override
  Future<bool> examClose(String sitting) async {
    if (failExamClose) {
      examCloseCalls++;
      throw StateError('exam close failed');
    }
    if (closed) throw StateError('examClose on a closed client');
    examCloseCalls++;
    examClosedSittings.add(sitting);
    if (examCloseGate != null) await examCloseGate!.future;
    if (expireOnExamClose) throw const PairingExpired();
    return examCloseReply;
  }

  @override
  Future<GradingCall> gradingSubmit(String requestJson) {
    callLog?.add('gradingSubmit');
    return _grading(requestJson);
  }

  @override
  Future<GradingCall> gradingRemediate(String requestJson) {
    callLog?.add('gradingRemediate');
    remediateRequests.add(requestJson);
    return _grading(requestJson);
  }

  @override
  Future<GradingCall> gradingGet() {
    callLog?.add('gradingGet');
    return _grading(null);
  }

  @override
  Future<bool> gradingClose(String sitting) async {
    callLog?.add('gradingClose');
    if (expireOnGrading) throw const PairingExpired();
    gradingClosedSittings.add(sitting);
    if (gradingCloseGate != null) await gradingCloseGate!.future;
    if (expireOnGradingClose) throw const PairingExpired();
    return gradingCloseReply;
  }

  Future<GradingCall> _grading(String? requestJson) async {
    final gate = gradingGates[_gradingCallsMade++];
    if (requestJson != null) gradingRequests.add(requestJson);
    if (gate != null) return gate.future;
    if (expireOnGrading) throw const PairingExpired();
    if (gradingReplies.isEmpty) return const GradingNoAnswer();
    final reply = gradingReplies[_gradingCall.clamp(0, gradingReplies.length - 1)];
    _gradingCall++;
    return reply;
  }

  @override
  Future<bool> generateStart(String url, {String? guidance}) async {
    generateStartCalled = true;
    generateStartedUrl = url;
    generateStartedGuidance = guidance;
    if (expireOnGenerateStart) throw const PairingExpired();
    return generateStartReply;
  }

  @override
  Future<RemoteGenerate?> generateGet() async {
    if (expireOnGenerateGet) throw const PairingExpired();
    if (generateGetReplies.isEmpty) return null;
    final call = _generateGetCall;
    _generateGetCall++;
    return generateGetReplies[call.clamp(0, generateGetReplies.length - 1)];
  }

  @override
  Future<void> generateClose() async => generateCloseCalls++;

  @override
  void close() => closed = true;
}

/// One `postCard` call as the fake saw it.
class PostCardCall {
  const PostCardCall({
    required this.deckId,
    required this.cardId,
    required this.front,
    required this.back,
  });

  final String deckId;
  final String cardId;
  final String front;
  final List<String> back;
}
