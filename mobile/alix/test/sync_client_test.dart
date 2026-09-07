// Pure-Dart tests for HttpSyncClient's wire behavior against an in-process
// dart:io HttpServer (loopback, port 0), mirroring server_client_test.dart.
// No RustLib, no real alix server. The JSON shapes mirror docs/API.md
// section 4.12 (SyncEntriesDto, SyncPushDto, SyncConflictDto, SyncRootDto);
// they are inlined here rather than read from tests/contracts/, which does
// not yet carry the sync corpus in this checkout.
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/server_client.dart' show PairingExpired, ServerConfig;
import 'package:alix_mobile/sync_client.dart';

void main() {
  group('HttpSyncClient', () {
    HttpServer? server;
    Directory? scratch;

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

    File targetFile() => File('${scratch!.path}/pulled.zip');

    setUp(() {
      scratch = Directory.systemTemp.createTempSync('alix-sync-client-');
    });

    tearDown(() async {
      await server?.close(force: true);
      server = null;
      final dir = scratch;
      if (dir != null && dir.existsSync()) dir.deleteSync(recursive: true);
      scratch = null;
    });

    group('entries', () {
      test('a normal reply parses root_id and every entry', () async {
        final s = await startServer((request) async {
          await request.drain<void>();
          await respondJson(request, 200, {
            'root_id': 'root-00000000000000000000000000',
            'entries': [
              {'name': 'Biology', 'kind': 'workspace', 'members': 2, 'unpacked_bytes': 4096},
            ],
          });
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);

        final result = await client.entries();

        expect(result.rootId, 'root-00000000000000000000000000');
        expect(
          result.entries,
          const [SyncEntry(name: 'Biology', kind: 'workspace', members: 2, unpackedBytes: 4096)],
        );
      });

      test('an empty entries list parses as empty, not an error', () async {
        final s = await startServer((request) async {
          await request.drain<void>();
          await respondJson(request, 200, {'root_id': 'root-a', 'entries': <Object>[]});
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);

        final result = await client.entries();

        expect(result.rootId, 'root-a');
        expect(result.entries, isEmpty);
      });

      test('a malformed row is skipped, valid rows still parse', () async {
        final s = await startServer((request) async {
          await request.drain<void>();
          await respondJson(request, 200, {
            'root_id': 'root-a',
            'entries': [
              {'name': 'Biology', 'kind': 'workspace', 'members': 2, 'unpacked_bytes': 4096},
              {'name': 'missing kind field'},
              'not even a map',
            ],
          });
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);

        final result = await client.entries();

        expect(
          result.entries,
          const [SyncEntry(name: 'Biology', kind: 'workspace', members: 2, unpackedBytes: 4096)],
        );
      });

      test('a 401 throws PairingExpired', () async {
        final s = await startServer((request) async {
          await request.drain<void>();
          request.response.statusCode = 401;
          await request.response.close();
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'stale'));
        addTearDown(client.close);

        expect(client.entries(), throwsA(isA<PairingExpired>()));
      });

      test('a 500 throws SyncTransportFailure', () async {
        final s = await startServer((request) async {
          await request.drain<void>();
          await respondJson(request, 500, {'error': 'root or catalog failure'});
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);

        expect(client.entries(), throwsA(isA<SyncTransportFailure>()));
      });
    });

    group('pull', () {
      test('writes the exact bytes to target and returns the Content-Length', () async {
        final bytes = List<int>.generate(5000, (i) => i % 256);
        final s = await startServer((request) async {
          await request.drain<void>();
          request.response.headers.contentType = ContentType('application', 'zip');
          request.response.contentLength = bytes.length;
          request.response.add(bytes);
          await request.response.close();
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);
        final target = targetFile();

        final written = await client.pull('Biology', target);

        expect(written, bytes.length);
        expect(target.readAsBytesSync(), bytes);
      });

      test('reports progress as bytes arrive', () async {
        final bytes = List<int>.filled(2000, 7);
        final s = await startServer((request) async {
          await request.drain<void>();
          request.response.contentLength = bytes.length;
          request.response.add(bytes);
          await request.response.close();
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);
        final target = targetFile();
        final progress = <(int, int)>[];

        await client.pull(
          'Biology',
          target,
          onProgress: (received, total) => progress.add((received, total)),
        );

        expect(progress, isNotEmpty);
        expect(progress.last, (bytes.length, bytes.length));
        for (final (received, total) in progress) {
          expect(total, bytes.length);
          expect(received, lessThanOrEqualTo(bytes.length));
        }
      });

      test('truncates an existing target file before writing', () async {
        final target = targetFile();
        target.writeAsStringSync('stale leftover content that is much longer than the new body');
        final bytes = [1, 2, 3];
        final s = await startServer((request) async {
          await request.drain<void>();
          request.response.contentLength = bytes.length;
          request.response.add(bytes);
          await request.response.close();
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);

        await client.pull('Biology', target);

        expect(target.readAsBytesSync(), bytes);
      });

      test('a body shorter than Content-Length throws and leaves no file behind', () async {
        // detachSocket lets the response declare a Content-Length and then
        // send fewer bytes than promised: HttpResponse.close() itself
        // enforces an exact byte count and refuses to send a short body.
        final s = await startServer((request) async {
          request.response.statusCode = 200;
          request.response.headers.contentType = ContentType('application', 'zip');
          request.response.headers.contentLength = 100;
          final socket = await request.response.detachSocket();
          socket.add(List<int>.filled(10, 65));
          await socket.flush();
          await socket.close();
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);
        final target = targetFile();

        await expectLater(
          () => client.pull('Biology', target),
          throwsA(isA<SyncTransportFailure>()),
        );
        expect(target.existsSync(), isFalse);
      });

      test('a 400 throws SyncTransportFailure and leaves no file behind', () async {
        final s = await startServer((request) async {
          await request.drain<void>();
          request.response.statusCode = 400;
          request.response.write('unknown entry');
          await request.response.close();
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);
        final target = targetFile();

        await expectLater(
          () => client.pull('nope', target),
          throwsA(isA<SyncTransportFailure>()),
        );
        expect(target.existsSync(), isFalse);
      });

      test('a 401 throws PairingExpired and leaves no file behind', () async {
        final s = await startServer((request) async {
          await request.drain<void>();
          request.response.statusCode = 401;
          await request.response.close();
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'stale'));
        addTearDown(client.close);
        final target = targetFile();

        await expectLater(
          () => client.pull('Biology', target),
          throwsA(isA<PairingExpired>()),
        );
        expect(target.existsSync(), isFalse);
      });
    });

    group('push', () {
      test('sends X-Alix-Root, X-Alix-Pulled-Revision, and the body verbatim', () async {
        String? seenPath;
        String? seenMethod;
        String? seenRoot;
        String? seenPulled;
        String? seenContentType;
        List<int>? seenBody;
        final s = await startServer((request) async {
          seenPath = request.uri.path;
          seenMethod = request.method;
          seenRoot = request.headers.value('X-Alix-Root');
          seenPulled = request.headers.value('X-Alix-Pulled-Revision');
          seenContentType = request.headers.contentType?.mimeType;
          seenBody = await request.fold<List<int>>(<int>[], (acc, chunk) => acc..addAll(chunk));
          await respondJson(request, 200, {'deck_id': 'deck-cells', 'revision': 8});
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);
        final document = utf8.encode('{"revision":7}');

        final result = await client.push(
          'deck-cells',
          document,
          rootId: 'root-00000000000000000000000000',
          pulledRevision: '7',
        );

        expect(seenMethod, 'POST');
        expect(seenPath, '/api/sync/push');
        expect(seenRoot, 'root-00000000000000000000000000');
        expect(seenPulled, '7');
        expect(seenContentType, 'application/json');
        expect(seenBody, document);
        expect(result, const SyncPushAccepted(deckId: 'deck-cells', revision: 8));
      });

      test('a none pulled revision is sent verbatim', () async {
        String? seenPulled;
        final s = await startServer((request) async {
          seenPulled = request.headers.value('X-Alix-Pulled-Revision');
          await request.drain<void>();
          await respondJson(request, 200, {'deck_id': 'deck-cells', 'revision': 1});
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);

        await client.push('deck-cells', utf8.encode('{}'), rootId: 'root-a', pulledRevision: 'none');

        expect(seenPulled, 'none');
      });

      test('404 maps to SyncPushNotServed', () async {
        final s = await startServer((request) async {
          await request.drain<void>();
          request.response.statusCode = 404;
          await request.response.close();
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);

        final result = await client.push('deck-x', utf8.encode('{}'), rootId: 'root-a', pulledRevision: 'none');

        expect(result, const SyncPushNotServed());
      });

      test('409 with populated fields maps to SyncPushConflict', () async {
        final s = await startServer((request) async {
          await request.drain<void>();
          await respondJson(request, 409, {
            'deck_id': 'deck-cells',
            'desktop_revision': 9,
            'pulled_revision': 7,
            'desktop_writer': {'device': 'desktop', 'at_ms': 42},
          });
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);

        final result = await client.push('deck-cells', utf8.encode('{}'), rootId: 'root-a', pulledRevision: '7');

        expect(
          result,
          const SyncPushConflict(
            deckId: 'deck-cells',
            desktopRevision: 9,
            pulledRevision: 7,
            desktopWriter: SyncWriter(device: 'desktop', atMs: 42),
          ),
        );
      });

      test('409 with null desktop fields (no existing document) maps with nulls', () async {
        final s = await startServer((request) async {
          await request.drain<void>();
          await respondJson(request, 409, {
            'deck_id': 'deck-cells',
            'desktop_revision': null,
            'pulled_revision': null,
            'desktop_writer': null,
          });
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);

        final result = await client.push('deck-cells', utf8.encode('{}'), rootId: 'root-a', pulledRevision: 'none');

        expect(
          result,
          const SyncPushConflict(
            deckId: 'deck-cells',
            desktopRevision: null,
            pulledRevision: null,
            desktopWriter: null,
          ),
        );
      });

      test('412 maps to SyncPushRootMismatch', () async {
        final s = await startServer((request) async {
          await request.drain<void>();
          await respondJson(request, 412, {'root_id': 'root-00000000000000000000000000'});
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);

        final result =
            await client.push('deck-cells', utf8.encode('{}'), rootId: 'root-stale', pulledRevision: '3');

        expect(result, const SyncPushRootMismatch(rootId: 'root-00000000000000000000000000'));
      });

      test('413 maps to SyncPushTooLarge', () async {
        final s = await startServer((request) async {
          await request.drain<void>();
          request.response.statusCode = 413;
          await request.response.close();
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);

        final result = await client.push('deck-cells', utf8.encode('{}'), rootId: 'root-a', pulledRevision: '3');

        expect(result, const SyncPushTooLarge());
      });

      test('400 maps to SyncPushRejected carrying status and body', () async {
        final s = await startServer((request) async {
          await request.drain<void>();
          request.response.statusCode = 400;
          request.response.write('malformed revision header');
          await request.response.close();
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);

        final result = await client.push('deck-cells', utf8.encode('{}'), rootId: 'root-a', pulledRevision: '3');

        expect(result, const SyncPushRejected(status: 400, body: 'malformed revision header'));
      });

      test('an unmapped status (503) also maps to SyncPushRejected', () async {
        final s = await startServer((request) async {
          await request.drain<void>();
          request.response.statusCode = 503;
          request.response.write('catalog owner unavailable');
          await request.response.close();
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'x'));
        addTearDown(client.close);

        final result = await client.push('deck-cells', utf8.encode('{}'), rootId: 'root-a', pulledRevision: '3');

        expect(result, const SyncPushRejected(status: 503, body: 'catalog owner unavailable'));
      });

      test('a 401 throws PairingExpired', () async {
        final s = await startServer((request) async {
          await request.drain<void>();
          request.response.statusCode = 401;
          await request.response.close();
        });
        final client = HttpSyncClient(ServerConfig(host: '127.0.0.1', port: s.port, token: 'stale'));
        addTearDown(client.close);

        expect(
          client.push('deck-cells', utf8.encode('{}'), rootId: 'root-a', pulledRevision: '3'),
          throwsA(isA<PairingExpired>()),
        );
      });
    });
  });
}
