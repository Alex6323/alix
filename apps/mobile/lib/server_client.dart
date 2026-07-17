// The phone's HTTP client for a paired desktop's `/api/remote/*` surface
// (the tutor and AI exam borrowed from the desktop's own AI backend), the
// `alix --lan` pairing-URL parser, the min-version gate, and the small
// typed DTOs mirroring the wire shapes pinned by tests/contracts/Remote*.json
// and documented in docs/API.md section 4.10 / section 6. dart:io only, no
// package:http: one small `HttpClient`, called rarely, does not earn a dep.
import 'dart:convert';
import 'dart:io';

/// Thrown when the paired server answers 401: the token this app holds no
/// longer matches what the server expects (a fresh `--lan` launch mints a
/// new one). Every [ServerClient] call can throw this instead of returning
/// its usual null/false; callers map it to a re-pair prompt. No other
/// failure mode throws.
class PairingExpired implements Exception {
  const PairingExpired();

  @override
  String toString() => 'PairingExpired: the paired server rejected this '
      "app's token";
}

/// A paired desktop: enough to reach it and prove who we are. No scheme
/// field on purpose, alix itself only ever speaks plain HTTP (see the
/// project's TLS stance); `parsePairingUrl` tolerates an `https://` input
/// without remembering it, since [HttpServerClient] always dials http.
class ServerConfig {
  const ServerConfig({required this.host, required this.port, required this.token});

  final String host;
  final int port;
  final String token;

  Map<String, dynamic> toJson() => {'host': host, 'port': port, 'token': token};

  /// Reconstructs from a settings map; null on any malformed shape (missing
  /// or wrong-typed field, or `json` not even a map). The caller treats
  /// that the same as "never paired", never throws.
  static ServerConfig? fromJson(dynamic json) {
    if (json is! Map) return null;
    final host = json['host'];
    final port = json['port'];
    final token = json['token'];
    if (host is! String || host.isEmpty) return null;
    if (port is! int) return null;
    if (token is! String || token.isEmpty) return null;
    return ServerConfig(host: host, port: port, token: token);
  }

  @override
  bool operator ==(Object other) =>
      other is ServerConfig && other.host == host && other.port == port && other.token == token;

  @override
  int get hashCode => Object.hash(host, port, token);

  // The token is a secret; keep it out of every print/interpolation path.
  @override
  String toString() => 'ServerConfig(host: $host, port: $port, token: <redacted>)';
}

/// Parses the URL `alix --lan` prints for pairing
/// (`http://<ip>:<port>/?token=<hex>`), tolerant of clipboard whitespace.
/// Anything that is not a well-formed http(s) URL with a non-empty host and
/// a non-empty `token` query parameter reads as null, never throws. IPv4,
/// hostnames, and bracketed IPv6 hosts all survive `Uri.parse`; a URL with
/// no port defaults to 80 (the server always prints one, but a client must
/// not crash on a hand-typed URL that omits it).
ServerConfig? parsePairingUrl(String input) {
  final trimmed = input.trim();
  if (trimmed.isEmpty) return null;
  Uri uri;
  try {
    uri = Uri.parse(trimmed);
  } on FormatException {
    return null;
  }
  if (uri.scheme != 'http' && uri.scheme != 'https') return null;
  if (uri.host.isEmpty) return null;
  final token = uri.queryParameters['token'];
  if (token == null || token.isEmpty) return null;
  final port = uri.hasPort ? uri.port : 80;
  return ServerConfig(host: uri.host, port: port, token: token);
}

/// Numeric, semver-shaped compare of `major.minor.patch`: a missing part
/// counts as zero, any `-`/`+` suffix is stripped before comparing. Not a
/// real semver parser, just enough for the min-server-version gate. Returns
/// negative/zero/positive like [Comparable.compare].
int compareVersions(String a, String b) {
  List<int> parts(String v) {
    final core = v.split(RegExp(r'[-+]')).first;
    final segments = core.split('.');
    return List.generate(
      3,
      (i) => i < segments.length ? int.tryParse(segments[i]) ?? 0 : 0,
    );
  }

  final pa = parts(a);
  final pb = parts(b);
  for (var i = 0; i < 3; i++) {
    final cmp = pa[i].compareTo(pb[i]);
    if (cmp != 0) return cmp;
  }
  return 0;
}

/// The oldest desktop crate version this app's remote surface understands
/// (the `/api/remote/*` routes shipped in 0.6.0). The pairing sheet refuses
/// an older server rather than call routes it does not have.
const minServerVersion = '0.6.0';

String? _asString(dynamic v) => v is String ? v : null;

int? _asInt(dynamic v) => v is num ? v.toInt() : null;

bool _asBool(dynamic v) => v == true;

List<String> _asStringList(dynamic v) => v is List ? v.whereType<String>().toList() : const [];

/// The card the tutor is discussing, sent whole on every call since the
/// server holds no session of its own for a remote turn. Mirrors
/// `RemoteCard` on the wire; a caller building one from the bridge's
/// generated `TutorCard` copies the same four fields across.
class TutorCardContext {
  const TutorCardContext({
    required this.subject,
    required this.front,
    required this.back,
    this.at,
  });

  final String subject;
  final String front;
  final List<String> back;
  final String? at;

  Map<String, dynamic> toJson() => {
        'subject': subject,
        'front': front,
        'back': back,
        'at': at,
      };
}

/// One prior tutor exchange, re-sent verbatim as part of `history` on every
/// call. Mirrors `RemoteTurn` on the wire.
class TutorTurn {
  const TutorTurn({required this.q, required this.a});

  final String q;
  final String a;

  Map<String, dynamic> toJson() => {'q': q, 'a': a};
}

/// A card drafted from a tutor exchange. Mirrors `DraftCardDto`.
class DraftCard {
  const DraftCard({required this.front, required this.back});

  final String front;
  final List<String> back;

  static DraftCard? fromJson(dynamic json) {
    if (json is! Map) return null;
    final front = _asString(json['front']);
    if (front == null) return null;
    return DraftCard(front: front, back: _asStringList(json['back']));
  }
}

/// The reply to a remote tutor call (`POST`/`GET /api/remote/ask`,
/// `POST /api/remote/ask/draft`). Mirrors `RemoteAskDto`; unknown wire
/// fields are ignored and absent ones read as their default, so an older
/// client survives a server that has grown new fields.
class RemoteAsk {
  const RemoteAsk({
    required this.thinking,
    this.answer,
    this.draft,
    this.note,
    this.error,
    this.elapsed,
  });

  final bool thinking;
  final String? answer;
  final DraftCard? draft;

  /// Condensed note lines from a note call. THREE distinct wire states,
  /// all preserved: absent/null (this reply is not a note result) reads as
  /// null here; `[]` (a note call that found nothing to save) reads as an
  /// empty list; `["a", "b"]` reads as those lines. Do not collapse the
  /// first two with `_asStringList` alone, it would turn absent into `[]`
  /// and lose the "not a note result" state the UI depends on.
  final List<String>? note;
  final String? error;
  final int? elapsed;

  static RemoteAsk fromJson(Map<String, dynamic> json) => RemoteAsk(
        thinking: _asBool(json['thinking']),
        answer: _asString(json['answer']),
        draft: DraftCard.fromJson(json['draft']),
        note: json['note'] is List ? _asStringList(json['note']) : null,
        error: _asString(json['error']),
        elapsed: _asInt(json['elapsed']),
      );
}

/// One graded exam answer within a `RemoteExam`. Mirrors `ExamGradeDto`;
/// `verdict` is uppercase (`PASS` | `PARTIAL` | `FAIL`) on the wire, passed
/// through as-is.
class RemoteExamGrade {
  const RemoteExamGrade({
    required this.question,
    required this.points,
    required this.answer,
    required this.verdict,
    required this.feedback,
    required this.missed,
  });

  final String question;
  final List<String> points;
  final String answer;
  final String verdict;
  final String feedback;
  final List<String> missed;

  static RemoteExamGrade? fromJson(dynamic json) {
    if (json is! Map) return null;
    final question = _asString(json['question']);
    final answer = _asString(json['answer']);
    final verdict = _asString(json['verdict']);
    final feedback = _asString(json['feedback']);
    if (question == null || answer == null || verdict == null || feedback == null) {
      return null;
    }
    return RemoteExamGrade(
      question: question,
      points: _asStringList(json['points']),
      answer: answer,
      verdict: verdict,
      feedback: feedback,
      missed: _asStringList(json['missed']),
    );
  }
}

/// A paired phone's AI exam sitting. Mirrors `RemoteExamDto`; unlike the
/// browser's `ExamDto` there is no server-side session, so this carries no
/// `total`/`current`/`question`/`answer`/`on_last`. `phase: "idle"` is the
/// baseline when no sitting is open.
class RemoteExam {
  const RemoteExam({
    required this.phase,
    required this.deck,
    required this.strictness,
    required this.questions,
    this.passed,
    required this.grades,
    required this.gaps,
    required this.canRemediate,
    this.cards,
    required this.isTrace,
    required this.thinking,
    this.elapsed,
    this.error,
  });

  final String phase;
  final String deck;
  final String strictness;
  final List<String> questions;
  final bool? passed;
  final List<RemoteExamGrade> grades;
  final List<String> gaps;
  final bool canRemediate;

  /// Deck-format text set only in the `remediated` phase: the client parses
  /// and stores these cards, the server never keeps them (the iron rule).
  final String? cards;

  /// A trace (compression) sitting vs a fact-deck sitting. Tolerant,
  /// defaults to false when absent (an older server or the idle baseline).
  final bool isTrace;
  final bool thinking;
  final int? elapsed;
  final String? error;

  static RemoteExam fromJson(Map<String, dynamic> json) => RemoteExam(
        phase: _asString(json['phase']) ?? 'idle',
        deck: _asString(json['deck']) ?? '',
        strictness: _asString(json['strictness']) ?? 'balanced',
        questions: _asStringList(json['questions']),
        passed: json['passed'] is bool ? json['passed'] as bool : null,
        grades: json['grades'] is List
            ? (json['grades'] as List).map(RemoteExamGrade.fromJson).whereType<RemoteExamGrade>().toList()
            : const [],
        gaps: _asStringList(json['gaps']),
        canRemediate: _asBool(json['can_remediate']),
        cards: _asString(json['cards']),
        isTrace: _asBool(json['is_trace']),
        thinking: _asBool(json['thinking']),
        elapsed: _asInt(json['elapsed']),
        error: _asString(json['error']),
      );
}

/// A paired desktop's deck generation from a URL (`POST`/`GET
/// /api/remote/generate`). Mirrors `RemoteGenerateDto`; unknown wire fields
/// are ignored and absent ones read as their default, so an older client
/// survives a server that has grown new fields. The iron rule: the finished
/// [deck] text is only ever read back here, this app places the file, the
/// server never does.
class RemoteGenerate {
  const RemoteGenerate({
    required this.phase,
    this.deck,
    this.filename,
    this.cards,
    this.elapsed,
    this.error,
  });

  /// `generating` | `done` | `error` (open set); `generating` is the safe
  /// baseline when absent.
  final String phase;

  /// The full generated deck text, set only in `done`.
  final String? deck;

  /// A suggested file name, set only in `done`; this app decides where and
  /// under what name to save it.
  final String? filename;
  final int? cards;
  final int? elapsed;
  final String? error;

  static RemoteGenerate fromJson(Map<String, dynamic> json) => RemoteGenerate(
        phase: _asString(json['phase']) ?? 'generating',
        deck: _asString(json['deck']),
        filename: _asString(json['filename']),
        cards: _asInt(json['cards']),
        elapsed: _asInt(json['elapsed']),
        error: _asString(json['error']),
      );
}

/// The phone's view of a paired desktop's AI backend: tutor and exam calls
/// over `/api/remote/*`. Behind an interface so the tutor and exam screens
/// (built on top of this) can fake it in tests, the same seam
/// [PlatformAccess] plays for platform channels.
///
/// Error surface (LOCKED, every method obeys it):
/// - success -> the value; a `postX`/`examX` call -> true on 2xx.
/// - HTTP 401 -> throws [PairingExpired]; callers map it to a re-pair
///   prompt.
/// - unreachable, timeout, refused, or non-HTTP garbage -> null (false for
///   a bool-returning call): failure reads as absence, never an exception.
/// - any other status (400, 403, 409, ...) -> false/null for v1; callers
///   treat it as a generic failed tap, there is no per-status exception.
abstract class ServerClient {
  /// The paired server's crate version (`GET /api/version`), or null if it
  /// cannot be reached or does not answer with JSON.
  Future<String?> version();

  /// The configured AI backend's display name (`GET /api/ask-info`,
  /// `AskInfoDto.backend`), or null on any failure; callers fall back to a
  /// generic label.
  Future<String?> backendName();

  /// `POST /api/remote/ask`: asks (or continues asking) the tutor about
  /// [card], re-sending the whole prior [history].
  Future<bool> postAsk(TutorCardContext card, List<TutorTurn> history, String question);

  /// `GET /api/remote/ask`: polls the in-flight or last-settled tutor turn.
  Future<RemoteAsk?> getAsk();

  /// `POST /api/remote/ask/draft`: distills [history] into a draft card.
  Future<bool> postDraft(TutorCardContext card, List<TutorTurn> history);

  /// `POST /api/remote/ask/note`: distills [history] into condensed note
  /// lines. The lines themselves come back on the shared ask slot: after
  /// this returns true, poll [getAsk] and read its `note`.
  Future<bool> postNote(TutorCardContext card, List<TutorTurn> history);

  /// `POST /api/remote/exam/start`: opens a sitting on [deck] (a bare name
  /// or `<workspace>/<file>`, resolved the same way `/api/select` does).
  Future<bool> examStart(String deck);

  /// `GET /api/remote/exam`: polls the open sitting.
  Future<RemoteExam?> examGet();

  /// `POST /api/remote/exam/grade`: submits every answer as one batch.
  Future<bool> examGrade(List<String> answers);

  /// `POST /api/remote/exam/remediate`: turns a failed, remediable result
  /// into cards (read back from the next `examGet()`'s `cards`).
  Future<bool> examRemediate();

  /// `POST /api/remote/exam/close`: drops the server's sitting slot.
  Future<void> examClose();

  /// `POST /api/remote/generate`: starts generating a deck from [url],
  /// optionally steered by [guidance]. Omits `guidance` from the body when
  /// null or empty.
  Future<bool> generateStart(String url, {String? guidance});

  /// `GET /api/remote/generate`: polls the in-flight or last-settled
  /// generation.
  Future<RemoteGenerate?> generateGet();

  /// `POST /api/remote/generate/close`: drops the server's generation slot.
  Future<void> generateClose();

  /// Releases any held resources (the underlying HTTP client).
  void close();
}

/// The real [ServerClient]: dart:io's `HttpClient` against a paired
/// [ServerConfig]. One instance holds one connection pool; call [close]
/// when done with it.
class HttpServerClient implements ServerClient {
  HttpServerClient(this.config) : _client = HttpClient()..connectionTimeout = const Duration(seconds: 3);

  final ServerConfig config;
  final HttpClient _client;

  static const _requestTimeout = Duration(seconds: 10);

  Uri _uri(String path) => Uri(scheme: 'http', host: config.host, port: config.port, path: path);

  /// Runs one call and returns its decoded JSON body, implementing the
  /// LOCKED error surface: a 401 throws [PairingExpired]; every other
  /// non-2xx-JSON outcome (unreachable, timed out, refused, malformed body,
  /// 400, 403, 409, ...) reads as null. [body], when present, is sent as a
  /// JSON object with an explicit `Content-Length` (never chunked, so the
  /// server's capped body reader always sees a clean end).
  Future<Map<String, dynamic>?> _call(String method, String path, [Object? body]) async {
    try {
      return await _attempt(method, path, body).timeout(_requestTimeout);
    } on PairingExpired {
      rethrow;
    } on Object {
      return null;
    }
  }

  Future<Map<String, dynamic>?> _attempt(String method, String path, Object? body) async {
    final request = await (method == 'GET' ? _client.getUrl(_uri(path)) : _client.postUrl(_uri(path)));
    request.headers.set(HttpHeaders.authorizationHeader, 'Bearer ${config.token}');
    if (body != null) {
      final bytes = utf8.encode(jsonEncode(body));
      request.headers.contentType = ContentType.json;
      request.contentLength = bytes.length;
      request.add(bytes);
    }
    final response = await request.close();
    final text = await response.transform(utf8.decoder).join();
    if (response.statusCode == 401) throw const PairingExpired();
    if (response.statusCode < 200 || response.statusCode >= 300) return null;
    if (text.isEmpty) return const {};
    final decoded = jsonDecode(text);
    return decoded is Map<String, dynamic> ? decoded : null;
  }

  Future<Map<String, dynamic>?> _get(String path) => _call('GET', path);

  Future<Map<String, dynamic>?> _post(String path, [Object? body]) => _call('POST', path, body);

  @override
  Future<String?> version() async => _asString((await _get('/api/version'))?['version']);

  @override
  Future<String?> backendName() async => _asString((await _get('/api/ask-info'))?['backend']);

  @override
  Future<bool> postAsk(TutorCardContext card, List<TutorTurn> history, String question) async {
    final json = await _post('/api/remote/ask', {
      'card': card.toJson(),
      'history': history.map((t) => t.toJson()).toList(),
      'question': question,
    });
    return json != null;
  }

  @override
  Future<RemoteAsk?> getAsk() async {
    final json = await _get('/api/remote/ask');
    return json == null ? null : RemoteAsk.fromJson(json);
  }

  @override
  Future<bool> postDraft(TutorCardContext card, List<TutorTurn> history) async {
    final json = await _post('/api/remote/ask/draft', {
      'card': card.toJson(),
      'history': history.map((t) => t.toJson()).toList(),
    });
    return json != null;
  }

  @override
  Future<bool> postNote(TutorCardContext card, List<TutorTurn> history) async {
    final json = await _post('/api/remote/ask/note', {
      'card': card.toJson(),
      'history': history.map((t) => t.toJson()).toList(),
    });
    return json != null;
  }

  @override
  Future<bool> examStart(String deck) async {
    final json = await _post('/api/remote/exam/start', {'deck': deck});
    return json != null;
  }

  @override
  Future<RemoteExam?> examGet() async {
    final json = await _get('/api/remote/exam');
    return json == null ? null : RemoteExam.fromJson(json);
  }

  @override
  Future<bool> examGrade(List<String> answers) async {
    final json = await _post('/api/remote/exam/grade', {'answers': answers});
    return json != null;
  }

  @override
  Future<bool> examRemediate() async {
    final json = await _post('/api/remote/exam/remediate');
    return json != null;
  }

  @override
  Future<void> examClose() async {
    await _post('/api/remote/exam/close');
  }

  @override
  Future<bool> generateStart(String url, {String? guidance}) async {
    final trimmedGuidance = guidance?.trim();
    final json = await _post('/api/remote/generate', {
      'url': url,
      if (trimmedGuidance != null && trimmedGuidance.isNotEmpty) 'guidance': trimmedGuidance,
    });
    return json != null;
  }

  @override
  Future<RemoteGenerate?> generateGet() async {
    final json = await _get('/api/remote/generate');
    return json == null ? null : RemoteGenerate.fromJson(json);
  }

  @override
  Future<void> generateClose() async {
    await _post('/api/remote/generate/close');
  }

  @override
  void close() => _client.close(force: true);
}
