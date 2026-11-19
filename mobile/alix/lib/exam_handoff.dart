import 'exam_sittings.dart';
import 'server_client.dart';

enum Handoff { none, fetched, closed, failed }

sealed class Claim {
  const Claim();
}

class ClaimClear extends Claim {
  const ClaimClear();
}

class ClaimBlocked extends Claim {
  const ClaimBlocked(this.record);

  final StartedSitting record;
}

class ClaimUnreachable extends Claim {
  const ClaimUnreachable();
}

enum _Step { none, close, fetch, failed }

class ExamHandoff {
  ExamHandoff({required this.sittings, required this.client, required this.heldSitting, required this.receive});

  final StartedSittings sittings;
  final ServerClient client;
  final String? Function(String root, String deckId) heldSitting;

  /// Writes the held file; throws when it cannot.
  final void Function(StartedSitting record, String carriedBody) receive;

  Future<Handoff> onReady(String sitting) async {
    final first = _decide(sitting);
    if (first.$1 != _Step.fetch) return _act(sitting, first, null);
    final body = await client.examCarried(sitting);
    if (body == null) return Handoff.failed;
    return _act(sitting, _decide(sitting), body);
  }

  Future<Claim> claimBeforeStart() async {
    final reply = await client.examGet();
    if (reply == null) return const ClaimUnreachable();
    if (reply.phase != 'ready') return const ClaimClear();
    final sitting = reply.sitting;
    if (sitting == null) return const ClaimUnreachable();
    final record = sittings.bySitting(sitting);
    if (await onReady(sitting) == Handoff.failed && record != null) return ClaimBlocked(record);
    return const ClaimClear();
  }

  Future<void> discardReady(StartedSitting record) async {
    await client.examClose(record.sitting);
    sittings.remove(record.root, record.deckId);
  }

  (_Step, StartedSitting?) _decide(String sitting) {
    final record = sittings.bySitting(sitting);
    if (record == null) return (_Step.none, null);
    if (record.fetched) return (_Step.close, record);
    final String? held;
    try {
      held = heldSitting(record.root, record.deckId);
    } on Object {
      return (_Step.failed, record);
    }
    if (held == sitting) return (_Step.close, record);
    if (held != null) return (_Step.failed, record);
    return (_Step.fetch, record);
  }

  Future<Handoff> _act(String sitting, (_Step, StartedSitting?) decision, String? body) async {
    switch (decision) {
      case (_Step.close, final record?):
        if (!record.fetched) _flagFetched(record);
        await client.examClose(sitting);
        return Handoff.closed;
      case (_Step.fetch, final record?) when body != null:
        try {
          receive(record, body);
        } on Object {
          return Handoff.failed;
        }
        _flagFetched(record);
        await client.examClose(sitting);
        return Handoff.fetched;
      case (_Step.failed, _):
        return Handoff.failed;
      case _:
        return Handoff.none;
    }
  }

  void _flagFetched(StartedSitting record) {
    record.fetched = true;
    sittings.save();
  }
}
