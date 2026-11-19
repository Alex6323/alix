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

class ExamHandoff {
  ExamHandoff({required this.sittings, required this.client, required this.heldSitting, required this.receive});

  final StartedSittings sittings;
  final ServerClient client;
  final String? Function(String root, String deckId) heldSitting;

  /// Writes the held file; throws when it cannot.
  final void Function(StartedSitting record, String carriedBody) receive;

  Future<Handoff> onReady(String sitting) async {
    final record = sittings.bySitting(sitting);
    if (record == null) return Handoff.none;
    if (record.fetched) return _close(sitting, Handoff.closed);
    final String? held;
    try {
      held = heldSitting(record.root, record.deckId);
    } on Object {
      return Handoff.failed;
    }
    if (held == sitting) {
      _flagFetched(record);
      return _close(sitting, Handoff.closed);
    }
    if (held != null) return Handoff.none;
    final body = await client.examCarried(sitting);
    if (body == null) return Handoff.failed;
    try {
      receive(record, body);
    } on Object {
      return Handoff.failed;
    }
    _flagFetched(record);
    return _close(sitting, Handoff.fetched);
  }

  Future<Claim> claimBeforeStart() async {
    final reply = await client.examGet();
    if (reply == null) return const ClaimUnreachable();
    final sitting = reply.sitting;
    if (reply.phase != 'ready' || sitting == null) return const ClaimClear();
    final record = sittings.bySitting(sitting);
    if (await onReady(sitting) == Handoff.failed && record != null) return ClaimBlocked(record);
    return const ClaimClear();
  }

  Future<void> discardReady(StartedSitting record) async {
    await client.examClose(record.sitting);
    sittings.remove(record.root, record.deckId);
  }

  void _flagFetched(StartedSitting record) {
    record.fetched = true;
    sittings.save();
  }

  Future<Handoff> _close(String sitting, Handoff result) async {
    await client.examClose(sitting);
    return result;
  }
}
