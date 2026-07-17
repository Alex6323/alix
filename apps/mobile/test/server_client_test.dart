// Pure-Dart tests for the pairing-URL parser, the version comparator, and
// HttpServerClient's wire behavior against an in-process dart:io HttpServer
// (loopback, port 0). No RustLib, no real alix server.
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/server_client.dart';

void main() {
  group('parsePairingUrl', () {
    test('the exact printed form parses to a config', () {
      expect(
        parsePairingUrl('http://192.168.1.5:7777/?token=abcdef1234567890'),
        const ServerConfig(host: '192.168.1.5', port: 7777, token: 'abcdef1234567890'),
      );
    });

    test('leading and trailing whitespace from a clipboard paste is tolerated', () {
      expect(
        parsePairingUrl('  \n http://192.168.1.5:7777/?token=abc123 \n\t '),
        const ServerConfig(host: '192.168.1.5', port: 7777, token: 'abc123'),
      );
    });

    test('a missing token query parameter is rejected', () {
      expect(parsePairingUrl('http://192.168.1.5:7777/'), isNull);
    });

    test('an empty token value is rejected', () {
      expect(parsePairingUrl('http://192.168.1.5:7777/?token='), isNull);
    });

    test('garbage input is rejected, never throws', () {
      expect(parsePairingUrl('this is not a url'), isNull);
      expect(parsePairingUrl(''), isNull);
      expect(parsePairingUrl('   '), isNull);
      expect(parsePairingUrl('ftp://192.168.1.5:7777/?token=abc'), isNull);
      expect(parsePairingUrl('http://[::1/broken?token=abc'), isNull);
    });

    test('an https URL is accepted', () {
      expect(
        parsePairingUrl('https://192.168.1.5:7777/?token=abc123'),
        const ServerConfig(host: '192.168.1.5', port: 7777, token: 'abc123'),
      );
    });

    test('a missing port defaults to 80', () {
      expect(
        parsePairingUrl('http://alix.local/?token=abc123'),
        const ServerConfig(host: 'alix.local', port: 80, token: 'abc123'),
      );
    });

    test('a bracketed IPv6 host survives', () {
      expect(
        parsePairingUrl('http://[::1]:7777/?token=abc123'),
        const ServerConfig(host: '::1', port: 7777, token: 'abc123'),
      );
    });
  });

  group('compareVersions', () {
    test('equal versions compare as zero', () {
      expect(compareVersions('0.6.0', '0.6.0'), 0);
    });

    test('a greater patch wins', () {
      expect(compareVersions('0.6.1', '0.6.0'), greaterThan(0));
      expect(compareVersions('0.6.0', '0.6.1'), lessThan(0));
    });

    test('a greater minor wins', () {
      expect(compareVersions('0.7.0', '0.6.9'), greaterThan(0));
    });

    test('a greater major wins', () {
      expect(compareVersions('1.0.0', '0.99.99'), greaterThan(0));
    });

    test('missing parts count as zero', () {
      expect(compareVersions('0.6', '0.6.0'), 0);
    });

    test('a pre-release or build suffix is ignored', () {
      expect(compareVersions('0.6.0-rc1', '0.6.0'), 0);
      expect(compareVersions('0.6.0+build5', '0.6.0'), 0);
    });

    test('comparison is numeric, not lexicographic', () {
      expect(compareVersions('10.0.0', '9.9.9'), greaterThan(0));
    });
  });

  group('HttpServerClient', () {
    HttpServer? server;

    Future<HttpServer> startServer(Future<void> Function(HttpRequest request) handle) async {
      final s = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      server = s;
      s.listen((request) => handle(request));
      return s;
    }

    Future<void> respondJson(HttpRequest request, int status, Object body) async {
      request.response.statusCode = status;
      request.response.headers.contentType = ContentType.json;
      request.response.write(jsonEncode(body));
      await request.response.close();
    }

    tearDown(() async {
      await server?.close(force: true);
      server = null;
    });

    test('the Bearer header is sent on version() and on a tutor POST', () async {
      final seen = <String>[];
      final s = await startServer((request) async {
        seen.add(request.headers.value(HttpHeaders.authorizationHeader) ?? '');
        await request.drain<void>();
        await respondJson(request, 200, {'version': '0.6.0'});
      });
      final client = HttpServerClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'secret-tok'));
      addTearDown(client.close);

      expect(await client.version(), '0.6.0');
      expect(
        await client.postAsk(
          const TutorCardContext(subject: 's', front: 'f', back: ['b']),
          const [],
          'why?',
        ),
        isTrue,
      );

      expect(seen, ['Bearer secret-tok', 'Bearer secret-tok']);
    });

    test('a dead port reads as null within about 3.5s, no exception thrown', () async {
      final probe = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
      final deadPort = probe.port;
      await probe.close();

      final client = HttpServerClient(ServerConfig(host: '127.0.0.1', port: deadPort, token: 'x'));
      addTearDown(client.close);

      final stopwatch = Stopwatch()..start();
      final version = await client.version();
      stopwatch.stop();

      expect(version, isNull);
      expect(stopwatch.elapsed, lessThan(const Duration(milliseconds: 3500)));
    });

    test('a 401 response throws PairingExpired', () async {
      final s = await startServer((request) async {
        await request.drain<void>();
        request.response.statusCode = 401;
        await request.response.close();
      });
      final client = HttpServerClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'stale'));
      addTearDown(client.close);

      expect(client.version(), throwsA(isA<PairingExpired>()));
    });

    test('a version body with extra unknown fields still parses', () async {
      final s = await startServer((request) async {
        await request.drain<void>();
        await respondJson(request, 200, {
          'version': '0.6.0',
          'surprise': 42,
          'nested': {'a': 1},
        });
      });
      final client = HttpServerClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
      addTearDown(client.close);

      expect(await client.version(), '0.6.0');
    });

    test('getAsk maps a corpus-shaped RemoteAskDto, including its draft', () async {
      final corpus = jsonDecode(File('../../tests/contracts/RemoteAskDto.done.json').readAsStringSync());
      final s = await startServer((request) async {
        await request.drain<void>();
        await respondJson(request, 200, corpus);
      });
      final client = HttpServerClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
      addTearDown(client.close);

      final ask = await client.getAsk();
      expect(ask, isNotNull);
      expect(ask!.thinking, isFalse);
      expect(ask.answer, 'so drops are deterministic');
      expect(ask.draft, isNotNull);
      expect(ask.draft!.front, 'Why does Rust use one owner per value?');
      expect(ask.draft!.back, ['so drops are deterministic', 'no GC needed']);
    });

    test('examGet maps the remediated corpus shape, including cards', () async {
      final corpus =
          jsonDecode(File('../../tests/contracts/RemoteExamDto.remediated.json').readAsStringSync());
      final s = await startServer((request) async {
        await request.drain<void>();
        await respondJson(request, 200, corpus);
      });
      final client = HttpServerClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
      addTearDown(client.close);

      final exam = await client.examGet();
      expect(exam, isNotNull);
      expect(exam!.phase, 'remediated');
      expect(exam.deck, 'rust.txt');
      expect(exam.cards, contains('Why does Rust use ownership?'));
      expect(exam.grades, hasLength(1));
      expect(exam.grades.single.verdict, 'FAIL');
      expect(exam.gaps, ['ownership and the GC-free memory model']);
      expect(exam.canRemediate, isFalse);
      expect(exam.passed, isFalse);
    });
  });
}
