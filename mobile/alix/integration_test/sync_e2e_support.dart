// Harness for sync_e2e_test.dart: locates and runs the REAL desktop `alix`
// binary from this checkout over a temp served folder, and drives its web
// API the way the browser does. Not a test file (no `_test.dart` suffix), so
// `flutter test integration_test` never picks it up on its own.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// The debug binary this checkout builds (`cargo build`), never the
/// installed `~/.cargo/bin/alix`, which can be days stale. Searched upward
/// from the test's working directory and from the running app bundle, since
/// which of the two is the repo's descendant depends on how `flutter test`
/// was invoked.
File desktopBinary() {
  final starts = <String>[
    Directory.current.path,
    File(Platform.resolvedExecutable).parent.path,
  ];
  for (final start in starts) {
    var dir = Directory(start).absolute;
    for (var up = 0; up < 12; up++) {
      final candidate = File('${dir.path}/target/debug/alix');
      if (candidate.existsSync()) return candidate;
      final parent = dir.parent;
      if (parent.path == dir.path) break;
      dir = parent;
    }
  }
  fail(
    'no target/debug/alix above ${starts.join(" or ")}; '
    'run `cargo build` in the repo root first',
  );
}

/// Runs one `alix` subcommand to completion, failing the test with the full
/// output when it does not exit 0.
void runAlix(List<String> arguments) {
  final result = Process.runSync(desktopBinary().path, arguments);
  if (result.exitCode != 0) {
    fail(
      'alix ${arguments.join(" ")} exited ${result.exitCode}\n'
      'stdout: ${result.stdout}\nstderr: ${result.stderr}',
    );
  }
}

/// A running `alix <folder> --port 0 --token …`: the desktop half of the
/// end-to-end run. [start] returns only once the server has announced its
/// port and answered `/api/version`; [stop] is safe to call twice, so a
/// tearDown can always run it.
class DesktopServer {
  DesktopServer._(this._process, this.folder, this.port, this.token, this.rootId);

  final Process _process;

  /// The served folder (the desktop's own library root).
  final String folder;
  final int port;
  final String token;

  /// The served folder's `root_id`, read from `GET /api/version`.
  final String rootId;

  final HttpClient _client = HttpClient()
    ..connectionTimeout = const Duration(seconds: 5);
  bool _stopped = false;

  static Future<DesktopServer> start({
    required String folder,
    required String configPath,
    required String token,
    Duration limit = const Duration(seconds: 30),
  }) async {
    // An empty config file, so the run never inherits the developer's own
    // `~/.config/alix/config.toml` (audience, ports, tokens).
    File(configPath).writeAsStringSync('');
    final process = await Process.start(desktopBinary().path, [
      folder,
      '--port',
      '0',
      '--token',
      token,
      '--config',
      configPath,
    ]);
    final announced = Completer<int>();
    final transcript = StringBuffer();
    process.stdout.transform(utf8.decoder).listen((chunk) {
      transcript.write(chunk);
      final match = RegExp(r'http://127\.0\.0\.1:(\d+)').firstMatch(chunk);
      if (match != null && !announced.isCompleted) {
        announced.complete(int.parse(match.group(1)!));
      }
    });
    process.stderr.transform(utf8.decoder).listen(transcript.write);

    final port = await announced.future.timeout(
      limit,
      onTimeout: () {
        process.kill();
        fail('alix never announced a port in ${limit.inSeconds}s: $transcript');
      },
    );

    final probe = DesktopServer._(process, folder, port, token, '');
    final version = await probe._get('/api/version');
    await probe._close();
    final rootId = version['root_id'];
    if (rootId is! String || rootId.isEmpty) {
      process.kill();
      fail('GET /api/version carried no root_id: $version');
    }
    return DesktopServer._(process, folder, port, token, rootId);
  }

  Future<void> stop() async {
    if (_stopped) return;
    _stopped = true;
    await _close();
    _process.kill();
    await _process.exitCode;
  }

  Future<void> _close() async {
    _client.close(force: true);
  }

  Uri _uri(String path) =>
      Uri(scheme: 'http', host: '127.0.0.1', port: port, path: path);

  void _authorize(HttpClientRequest request) {
    request.headers.set(HttpHeaders.authorizationHeader, 'Bearer $token');
  }

  Future<Map<String, dynamic>> _get(String path) async {
    final request = await _client.getUrl(_uri(path));
    _authorize(request);
    final response = await request.close();
    final body = await response.transform(utf8.decoder).join();
    if (response.statusCode != 200) {
      fail('GET $path answered ${response.statusCode}: $body');
    }
    return jsonDecode(body) as Map<String, dynamic>;
  }

  Future<Map<String, dynamic>> post(String path, Object body,
      {Map<String, String> headers = const {}}) async {
    final request = await _client.postUrl(_uri(path));
    _authorize(request);
    request.headers.contentType = ContentType.json;
    headers.forEach(request.headers.set);
    final bytes = utf8.encode(jsonEncode(body));
    request.contentLength = bytes.length;
    request.add(bytes);
    final response = await request.close();
    final text = await response.transform(utf8.decoder).join();
    if (response.statusCode != 200) {
      fail('POST $path answered ${response.statusCode}: $text');
    }
    return text.isEmpty
        ? <String, dynamic>{}
        : jsonDecode(text) as Map<String, dynamic>;
  }

  /// One desktop review of [deck] at Reconstruct depth, the depth a phone
  /// session at Recall leaves unscheduled: select, grade, deselect, exactly
  /// what the browser sends (docs/API.md section 5, "Review session"). Every
  /// transition rewrites the deck's progress document, so this is what moves
  /// the desktop revision under a paired phone.
  Future<void> reviewOnDesktop(String deck) async {
    // The picker listing first, as a browser does: it is the only request
    // that re-reads progress documents a paired push replaced under the
    // running server (`revalidate_progress_view`, src/serve/study.rs), so
    // without it the desktop session would build on pre-push state.
    await _get('/api/decks');
    final selected = await post('/api/select', {
      'deck': deck,
      'depth': 'reconstruct',
    });
    expect(
      selected['phase'],
      'review',
      reason:
          'the desktop needs a due card to grade; /api/select answered phase '
          '${selected['phase']} for $deck at reconstruct',
    );
    await post(
      '/api/grade',
      {'grade': 'passed'},
      headers: {
        'X-Alix-Study-Revision': '${selected['study_revision']}',
      },
    );
    await post('/api/deselect', const <String, dynamic>{});
  }
}

/// A deck's progress document as the store keeps it
/// (`<store root>/.alix/progress/<deck-id>.json`); null when none exists yet.
Map<String, dynamic>? progressDocument(String storeRoot, String deckId) {
  final file = File('$storeRoot/.alix/progress/$deckId.json');
  if (!file.existsSync()) return null;
  return jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
}

/// The `id:` an initialized deck file carries in its frontmatter.
String deckIdOf(String deckPath) {
  final match = RegExp(r'^id:\s*"(deck-[a-z0-9]+)"', multiLine: true)
      .firstMatch(File(deckPath).readAsStringSync());
  if (match == null) {
    fail('$deckPath carries no frontmatter deck id; was `alix deck init` run?');
  }
  return match.group(1)!;
}

/// Pumps real frames until [ready] holds, failing at [limit] rather than
/// hanging. The only waiting shape this suite uses: no bare delay ever
/// stands in for a condition.
Future<void> waitUntil(
  WidgetTester tester,
  String what,
  FutureOr<bool> Function() ready, {
  Duration limit = const Duration(seconds: 40),
}) async {
  final deadline = DateTime.now().add(limit);
  while (true) {
    await tester.pump(const Duration(milliseconds: 100));
    if (await ready()) return;
    if (DateTime.now().isAfter(deadline)) {
      fail('timed out after ${limit.inSeconds}s waiting for $what');
    }
  }
}
