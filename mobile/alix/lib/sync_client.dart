// The phone's HTTP client for a paired desktop's `/api/sync/*` surface (the
// picker-entry pull/push transport, docs/API.md section 4.12): listing what
// the desktop serves, pulling one entry's zip, and pushing one deck's
// progress document. dart:io only, no package:http, mirroring
// server_client.dart's own stance.
import 'dart:convert';
import 'dart:io';

import 'package:alix_mobile/server_client.dart' show PairingExpired, ServerConfig;

String? _asString(dynamic v) => v is String ? v : null;

int? _asInt(dynamic v) => v is num ? v.toInt() : null;

/// A wire response that did not match `docs/API.md`'s contract: a non-200
/// status the caller has no dedicated mapping for, a malformed success body,
/// or (for [SyncClient.pull]) a body shorter or longer than the
/// `Content-Length` header promised.
class SyncTransportFailure implements Exception {
  const SyncTransportFailure(this.status, this.body);

  final int status;
  final String body;

  @override
  String toString() => 'SyncTransportFailure: status $status: $body';
}

/// One entry `GET /api/sync/entries` lists. Mirrors `SyncEntryDto`.
class SyncEntry {
  const SyncEntry({
    required this.name,
    required this.kind,
    required this.members,
    required this.unpackedBytes,
  });

  final String name;

  /// Exactly `workspace` or `deck`.
  final String kind;
  final int members;
  final int unpackedBytes;

  static SyncEntry? fromJson(dynamic json) {
    if (json is! Map) return null;
    final name = _asString(json['name']);
    final kind = _asString(json['kind']);
    final members = _asInt(json['members']);
    final unpackedBytes = _asInt(json['unpacked_bytes']);
    if (name == null || kind == null || members == null || unpackedBytes == null) return null;
    return SyncEntry(name: name, kind: kind, members: members, unpackedBytes: unpackedBytes);
  }

  @override
  bool operator ==(Object other) =>
      other is SyncEntry &&
      other.name == name &&
      other.kind == kind &&
      other.members == members &&
      other.unpackedBytes == unpackedBytes;

  @override
  int get hashCode => Object.hash(name, kind, members, unpackedBytes);
}

/// The reply to `GET /api/sync/entries`. Mirrors `SyncEntriesDto`.
class SyncEntries {
  const SyncEntries({required this.rootId, required this.entries});

  final String rootId;
  final List<SyncEntry> entries;

  @override
  bool operator ==(Object other) {
    if (other is! SyncEntries || other.rootId != rootId) return false;
    if (other.entries.length != entries.length) return false;
    for (var i = 0; i < entries.length; i++) {
      if (other.entries[i] != entries[i]) return false;
    }
    return true;
  }

  @override
  int get hashCode => Object.hash(rootId, Object.hashAll(entries));
}

/// A disk document's writer, `{device, at_ms}`. Mirrors `Writer`.
class SyncWriter {
  const SyncWriter({required this.device, required this.atMs});

  final String device;
  final int atMs;

  static SyncWriter? fromJson(dynamic json) {
    if (json is! Map) return null;
    final device = _asString(json['device']);
    final atMs = _asInt(json['at_ms']);
    if (device == null || atMs == null) return null;
    return SyncWriter(device: device, atMs: atMs);
  }

  @override
  bool operator ==(Object other) => other is SyncWriter && other.device == device && other.atMs == atMs;

  @override
  int get hashCode => Object.hash(device, atMs);
}

/// The outcome of `POST /api/sync/push`, one case per status this app has a
/// dedicated mapping for (docs/API.md section 4.12's push status table).
sealed class SyncPushResult {
  const SyncPushResult();
}

/// 200: `SyncPushDto`, the push committed.
class SyncPushAccepted extends SyncPushResult {
  const SyncPushAccepted({required this.deckId, required this.revision});

  final String deckId;
  final int revision;

  @override
  bool operator ==(Object other) =>
      other is SyncPushAccepted && other.deckId == deckId && other.revision == revision;

  @override
  int get hashCode => Object.hash(deckId, revision);
}

/// 409: `SyncConflictDto`, the disk revision moved past what was pulled.
class SyncPushConflict extends SyncPushResult {
  const SyncPushConflict({
    required this.deckId,
    this.desktopRevision,
    this.pulledRevision,
    this.desktopWriter,
  });

  final String deckId;
  final int? desktopRevision;
  final int? pulledRevision;
  final SyncWriter? desktopWriter;

  @override
  bool operator ==(Object other) =>
      other is SyncPushConflict &&
      other.deckId == deckId &&
      other.desktopRevision == desktopRevision &&
      other.pulledRevision == pulledRevision &&
      other.desktopWriter == desktopWriter;

  @override
  int get hashCode => Object.hash(deckId, desktopRevision, pulledRevision, desktopWriter);
}

/// 412: `SyncRootDto`, the served root changed since the last pull.
class SyncPushRootMismatch extends SyncPushResult {
  const SyncPushRootMismatch({required this.rootId});

  final String rootId;

  @override
  bool operator ==(Object other) => other is SyncPushRootMismatch && other.rootId == rootId;

  @override
  int get hashCode => rootId.hashCode;
}

/// 404: no served entry owns the pushed deck id.
class SyncPushNotServed extends SyncPushResult {
  const SyncPushNotServed();

  @override
  bool operator ==(Object other) => other is SyncPushNotServed;

  @override
  int get hashCode => (SyncPushNotServed).hashCode;
}

/// 413: the document exceeds the sync-push cap.
class SyncPushTooLarge extends SyncPushResult {
  const SyncPushTooLarge();

  @override
  bool operator ==(Object other) => other is SyncPushTooLarge;

  @override
  int get hashCode => (SyncPushTooLarge).hashCode;
}

/// 400 or any other status this app has no dedicated mapping for.
class SyncPushRejected extends SyncPushResult {
  const SyncPushRejected({required this.status, required this.body});

  final int status;
  final String body;

  @override
  bool operator ==(Object other) =>
      other is SyncPushRejected && other.status == status && other.body == body;

  @override
  int get hashCode => Object.hash(status, body);
}

/// The phone's view of a paired desktop's `/api/sync/*` transport: list what
/// it serves, pull one entry's zip, push one deck's progress document.
/// Behind an interface, the same seam [ServerClient] plays for the remote
/// tutor/exam/generate surface, so a caller can fake it in tests.
abstract class SyncClient {
  /// `GET /api/sync/entries`: what the paired desktop currently serves.
  /// Throws [PairingExpired] on 401, [SyncTransportFailure] on any other
  /// non-200 status or a malformed body.
  Future<SyncEntries> entries();

  /// `GET /api/sync/pull?entry=<name>`: streams the entry's zip into
  /// [target] (created or truncated). Reads `Content-Length` before the
  /// body and returns it; throws [SyncTransportFailure] when the body
  /// length differs from it, or on any non-200 status, leaving no partial
  /// file behind. Throws [PairingExpired] on 401.
  Future<int> pull(
    String entry,
    File target, {
    void Function(int received, int total)? onProgress,
  });

  /// `POST /api/sync/push?deck=<deckId>`: pushes [document]'s raw bytes,
  /// asserting [rootId] (`X-Alix-Root`) and [pulledRevision]
  /// (`X-Alix-Pulled-Revision`, already formatted as `none` or a decimal by
  /// the caller). Throws [PairingExpired] on 401; every other status maps to
  /// a [SyncPushResult] case, never an exception.
  Future<SyncPushResult> push(
    String deckId,
    List<int> document, {
    required String rootId,
    required String pulledRevision,
  });
}

/// The real [SyncClient]: dart:io's `HttpClient` against a paired
/// [ServerConfig]. One instance holds one connection pool; call [close] when
/// done with it.
class HttpSyncClient implements SyncClient {
  HttpSyncClient(this.config) : _client = HttpClient()..connectionTimeout = const Duration(seconds: 3);

  final ServerConfig config;
  final HttpClient _client;

  Uri _uri(String path, [Map<String, String>? query]) => Uri(
        scheme: config.scheme,
        host: config.host,
        port: config.port,
        path: path,
        queryParameters: query,
      );

  void _authorize(HttpClientRequest request) {
    request.headers.set(HttpHeaders.authorizationHeader, 'Bearer ${config.token}');
  }

  @override
  Future<SyncEntries> entries() async {
    final request = await _client.getUrl(_uri('/api/sync/entries'));
    _authorize(request);
    final response = await request.close();
    if (response.statusCode == 401) {
      await response.drain<void>();
      throw const PairingExpired();
    }
    final body = await response.transform(utf8.decoder).join();
    if (response.statusCode != 200) {
      throw SyncTransportFailure(response.statusCode, body);
    }
    dynamic json;
    try {
      json = jsonDecode(body);
    } on FormatException {
      throw SyncTransportFailure(response.statusCode, body);
    }
    final rootId = json is Map ? _asString(json['root_id']) : null;
    if (rootId == null) throw SyncTransportFailure(response.statusCode, body);
    final rawEntries = json['entries'];
    final entries = rawEntries is List
        ? rawEntries.map(SyncEntry.fromJson).whereType<SyncEntry>().toList()
        : <SyncEntry>[];
    return SyncEntries(rootId: rootId, entries: entries);
  }

  @override
  Future<int> pull(
    String entry,
    File target, {
    void Function(int received, int total)? onProgress,
  }) async {
    final request = await _client.getUrl(_uri('/api/sync/pull', {'entry': entry}));
    _authorize(request);
    final response = await request.close();
    if (response.statusCode == 401) {
      await response.drain<void>();
      throw const PairingExpired();
    }
    if (response.statusCode != 200) {
      final body = await response.transform(utf8.decoder).join();
      throw SyncTransportFailure(response.statusCode, body);
    }
    final total = response.contentLength;
    var received = 0;
    final sink = target.openWrite();
    try {
      await for (final chunk in response) {
        received += chunk.length;
        sink.add(chunk);
        onProgress?.call(received, total);
      }
      await sink.close();
    } on Object catch (error) {
      await sink.close();
      if (await target.exists()) await target.delete();
      throw SyncTransportFailure(200, 'transfer failed after $received of $total bytes: $error');
    }
    if (received != total) {
      if (await target.exists()) await target.delete();
      throw SyncTransportFailure(200, 'received $received bytes, Content-Length said $total');
    }
    return received;
  }

  @override
  Future<SyncPushResult> push(
    String deckId,
    List<int> document, {
    required String rootId,
    required String pulledRevision,
  }) async {
    final request = await _client.postUrl(_uri('/api/sync/push', {'deck': deckId}));
    _authorize(request);
    request.headers.set('X-Alix-Root', rootId);
    request.headers.set('X-Alix-Pulled-Revision', pulledRevision);
    request.headers.contentType = ContentType.json;
    request.contentLength = document.length;
    request.add(document);
    final response = await request.close();
    switch (response.statusCode) {
      case 401:
        await response.drain<void>();
        throw const PairingExpired();
      case 200:
        final (text, json) = await _readBody(response);
        final map = json is Map ? json : null;
        final id = map == null ? null : _asString(map['deck_id']);
        final revision = map == null ? null : _asInt(map['revision']);
        if (id == null || revision == null) throw SyncTransportFailure(200, text);
        return SyncPushAccepted(deckId: id, revision: revision);
      case 404:
        await response.drain<void>();
        return const SyncPushNotServed();
      case 409:
        final (text, json) = await _readBody(response);
        if (json is! Map) throw SyncTransportFailure(409, text);
        final conflictDeckId = _asString(json['deck_id']);
        if (conflictDeckId == null) throw SyncTransportFailure(409, text);
        return SyncPushConflict(
          deckId: conflictDeckId,
          desktopRevision: _asInt(json['desktop_revision']),
          pulledRevision: _asInt(json['pulled_revision']),
          desktopWriter: SyncWriter.fromJson(json['desktop_writer']),
        );
      case 412:
        final (text, json) = await _readBody(response);
        final map = json is Map ? json : null;
        final mismatchedRoot = map == null ? null : _asString(map['root_id']);
        if (mismatchedRoot == null) throw SyncTransportFailure(412, text);
        return SyncPushRootMismatch(rootId: mismatchedRoot);
      case 413:
        await response.drain<void>();
        return const SyncPushTooLarge();
      default:
        final body = await response.transform(utf8.decoder).join();
        return SyncPushRejected(status: response.statusCode, body: body);
    }
  }

  Future<(String, dynamic)> _readBody(HttpClientResponse response) async {
    final text = await response.transform(utf8.decoder).join();
    try {
      return (text, jsonDecode(text));
    } on FormatException {
      return (text, null);
    }
  }

  void close() => _client.close(force: true);
}
