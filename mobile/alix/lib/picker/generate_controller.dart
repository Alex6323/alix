import 'dart:async';

import 'package:flutter/foundation.dart';

import 'package:alix_mobile/server_client.dart';

abstract interface class GeneratePollTimer {
  void cancel();
}

typedef GeneratePollScheduler =
    GeneratePollTimer Function(
      Duration delay,
      Future<void> Function() callback,
    );

class GenerateController extends ChangeNotifier {
  factory GenerateController({
    required ServerClient client,
    Duration pollInterval = const Duration(milliseconds: 400),
    GeneratePollScheduler? schedulePoll,
  }) {
    return GenerateController._(
      client,
      pollInterval,
      schedulePoll ?? _defaultSchedulePoll,
    );
  }

  GenerateController._(this._client, this.pollInterval, this._schedulePoll);

  final ServerClient _client;
  final Duration pollInterval;
  final GeneratePollScheduler _schedulePoll;

  GeneratePollTimer? _pollTimer;
  bool _busy = false;
  String? _message;
  int? _elapsed;
  RemoteGenerate? _completed;
  bool _handedOff = false;
  bool _disposed = false;

  bool get busy => _busy;
  String? get message => _message;
  int? get elapsed => _elapsed;
  RemoteGenerate? get completed => _completed;

  Future<void> submit({required String url, required String guidance}) async {
    final trimmedUrl = url.trim();
    final scheme = Uri.tryParse(trimmedUrl)?.scheme;
    if (scheme != 'http' && scheme != 'https') {
      _fail('alix can only generate from an http:// or https:// URL.');
      return;
    }
    _pollTimer?.cancel();
    _pollTimer = null;
    _busy = true;
    _message = null;
    _elapsed = null;
    _completed = null;
    notifyListeners();

    final trimmedGuidance = guidance.trim();
    bool started;
    try {
      started = await _client.generateStart(
        trimmedUrl,
        guidance: trimmedGuidance.isEmpty ? null : trimmedGuidance,
      );
    } on PairingExpired {
      _fail('Pairing expired. Pair again from Settings → Connected devices.');
      return;
    }
    if (_disposed) return;
    if (!started) {
      _fail('The desktop refused to generate this deck.');
      return;
    }
    await _poll();
  }

  RemoteGenerate handoff() {
    final completed = _completed;
    if (completed == null) throw StateError('generation is not complete');
    _handedOff = true;
    return completed;
  }

  Future<void> _poll() async {
    RemoteGenerate? result;
    try {
      result = await _client.generateGet();
    } on PairingExpired {
      _fail('Pairing expired. Pair again from Settings → Connected devices.');
      return;
    }
    if (_disposed) return;
    if (result == null) {
      _fail('Lost contact with the desktop.');
      return;
    }
    if (result.phase == 'error') {
      _fail(result.error ?? 'The desktop failed to generate this deck.');
      return;
    }
    if (result.phase == 'done' &&
        result.deck != null &&
        result.filename != null) {
      _completed = result;
      notifyListeners();
      return;
    }
    _elapsed = result.elapsed;
    notifyListeners();
    _pollTimer = _schedulePoll(pollInterval, _poll);
  }

  void _fail(String message) {
    if (_disposed) return;
    _busy = false;
    _message = message;
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _pollTimer?.cancel();
    if (!_handedOff) {
      unawaited(_client.generateClose().catchError((_) {}));
      _client.close();
    }
    super.dispose();
  }
}

GeneratePollTimer _defaultSchedulePoll(
  Duration delay,
  Future<void> Function() callback,
) {
  return _DartPollHandle(Timer(delay, callback));
}

class _DartPollHandle implements GeneratePollTimer {
  const _DartPollHandle(this._timer);

  final Timer _timer;

  @override
  void cancel() => _timer.cancel();
}
