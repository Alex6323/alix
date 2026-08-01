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
    this.expireOnVersion = false,
    this.backendReply = 'Claude',
    List<bool>? postAskReplies,
    this.expireOnPostAsk = false,
    this.postAskGate,
    List<RemoteAsk>? getAskReplies,
    List<bool>? postDraftReplies,
    List<bool>? postNoteReplies,
    this.examStartReply = true,
    this.expireOnExamStart = false,
    List<RemoteExam>? examGetReplies,
    this.expireOnExamGet = false,
    Map<int, Completer<RemoteExam?>>? examGetGates,
    this.nullFirstExamGetCalls = 0,
    List<bool>? examGradeReplies,
    this.expireOnExamGrade = false,
    List<bool>? examRemediateReplies,
    this.expireOnExamRemediate = false,
    this.generateStartReply = true,
    this.expireOnGenerateStart = false,
    List<RemoteGenerate>? generateGetReplies,
    this.expireOnGenerateGet = false,
  })  : postAskReplies = postAskReplies ?? const [true],
        getAskReplies = getAskReplies ?? const [],
        postDraftReplies = postDraftReplies ?? const [true],
        postNoteReplies = postNoteReplies ?? const [true],
        examGetReplies = examGetReplies ?? const [],
        examGetGates = examGetGates ?? const {},
        examGradeReplies = examGradeReplies ?? const [true],
        examRemediateReplies = examRemediateReplies ?? const [true],
        generateGetReplies = generateGetReplies ?? const [];

  // ── probe (version / backendName) ────────────────────────────────────
  final String? versionReply;
  final bool expireOnVersion;
  final String? backendReply;

  // ── tutor (postAsk / getAsk / postDraft / postNote) ────────────────────
  final List<bool> postAskReplies;
  final bool expireOnPostAsk;

  /// When set, `postAsk` parks on this until the test completes it: the
  /// "request still in flight while the sheet is dismissed" lever.
  final Completer<bool>? postAskGate;
  final List<RemoteAsk> getAskReplies;
  final List<bool> postDraftReplies;
  final List<bool> postNoteReplies;

  // ── exam (examStart / examGet / examGrade / examRemediate) ─────────────
  final bool examStartReply;
  final bool expireOnExamStart;
  final List<RemoteExam> examGetReplies;
  final bool expireOnExamGet;

  /// Parks a specific (0-based) `examGet()` call on a completer instead of
  /// answering immediately: the lever for putting two polls in flight at
  /// once (a periodic tick firing again before the prior call's own
  /// cancellation lands).
  final Map<int, Completer<RemoteExam?>> examGetGates;

  /// How many leading `examGet()` calls return null regardless of
  /// [examGetReplies], simulating a slow sitting start before the real
  /// replies begin.
  final int nullFirstExamGetCalls;
  final List<bool> examGradeReplies;
  final bool expireOnExamGrade;
  final List<bool> examRemediateReplies;
  final bool expireOnExamRemediate;

  // ── generate (generateStart / generateGet) ──────────────────────────────
  final bool generateStartReply;
  final bool expireOnGenerateStart;
  final List<RemoteGenerate> generateGetReplies;
  final bool expireOnGenerateGet;

  // ── spies ────────────────────────────────────────────────────────────
  final List<List<TutorTurn>> postAskHistories = [];
  final List<List<TutorTurn>> postDraftHistories = [];
  final List<List<TutorTurn>> postNoteHistories = [];

  /// The deck argument of the most recent `examStart` call.
  String? startedDeck;

  bool generateStartCalled = false;
  String? generateStartedUrl;
  String? generateStartedGuidance;
  final List<List<String>> gradeAnswers = [];
  int examCloseCalls = 0;
  int generateCloseCalls = 0;
  bool closed = false;

  int _askCall = 0;
  int _pollCall = 0;
  int _draftCall = 0;
  int _noteCall = 0;
  int _examGetCall = 0;
  int _gradeCall = 0;
  int _remediateCall = 0;
  int _generateGetCall = 0;

  @override
  Future<String?> version() async {
    if (expireOnVersion) throw const PairingExpired();
    return versionReply;
  }

  @override
  Future<String?> backendName() async => backendReply;

  @override
  Future<bool> postAsk(TutorCardContext card, List<TutorTurn> history, String question) async {
    postAskHistories.add(history);
    if (expireOnPostAsk) throw const PairingExpired();
    if (postAskGate != null) return postAskGate!.future;
    final reply = postAskReplies[_askCall.clamp(0, postAskReplies.length - 1)];
    _askCall++;
    return reply;
  }

  @override
  Future<RemoteAsk?> getAsk() async {
    if (getAskReplies.isEmpty) return null;
    final reply = getAskReplies[_pollCall.clamp(0, getAskReplies.length - 1)];
    _pollCall++;
    return reply;
  }

  @override
  Future<bool> postDraft(TutorCardContext card, List<TutorTurn> history) async {
    postDraftHistories.add(history);
    final reply = postDraftReplies[_draftCall.clamp(0, postDraftReplies.length - 1)];
    _draftCall++;
    return reply;
  }

  @override
  Future<bool> postNote(TutorCardContext card, List<TutorTurn> history) async {
    postNoteHistories.add(history);
    final reply = postNoteReplies[_noteCall.clamp(0, postNoteReplies.length - 1)];
    _noteCall++;
    return reply;
  }

  @override
  Future<bool> examStart(String deck) async {
    startedDeck = deck;
    if (expireOnExamStart) throw const PairingExpired();
    return examStartReply;
  }

  @override
  Future<RemoteExam?> examGet() async {
    if (expireOnExamGet) throw const PairingExpired();
    final call = _examGetCall;
    _examGetCall++;
    final gate = examGetGates[call];
    if (gate != null) return gate.future;
    if (call < nullFirstExamGetCalls) return null;
    if (examGetReplies.isEmpty) return null;
    final repliesCall = call - nullFirstExamGetCalls;
    return examGetReplies[repliesCall.clamp(0, examGetReplies.length - 1)];
  }

  @override
  Future<bool> examGrade(List<String> answers) async {
    gradeAnswers.add(answers);
    if (expireOnExamGrade) throw const PairingExpired();
    final reply = examGradeReplies[_gradeCall.clamp(0, examGradeReplies.length - 1)];
    _gradeCall++;
    return reply;
  }

  @override
  Future<bool> examRemediate() async {
    if (expireOnExamRemediate) throw const PairingExpired();
    final reply = examRemediateReplies[_remediateCall.clamp(0, examRemediateReplies.length - 1)];
    _remediateCall++;
    return reply;
  }

  @override
  Future<void> examClose() async => examCloseCalls++;

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
