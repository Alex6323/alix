import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/exam_handoff.dart';
import 'package:alix_mobile/exam_sittings.dart';
import 'package:alix_mobile/server_client.dart';

import 'support/fake_server_client.dart';

StartedSitting _sitting({
  String root = 'root-a',
  String deckId = 'deck-1',
  String sitting = 'sitting-1',
  bool fetched = false,
}) => StartedSitting(
  root: root,
  deckId: deckId,
  deckPath: 'decks/$deckId.md',
  title: 'Deck $deckId',
  sitting: sitting,
  announced: false,
  fetched: fetched,
  start: ExamSittingStart(
    deckId: deckId,
    cards: null,
    eligible: true,
    sourced: 'xxh64-0123456789abcdef',
    unsourcedCards: const [],
    unsourcedTopics: const [],
  ),
);

RemoteExam _ready(String sitting) => RemoteExam(sitting: sitting, phase: 'ready', deck: 'Deck', thinking: false);

class _Desk extends FakeServerClient {
  _Desk(this.log, this.flagOnDisk);

  final List<String> log;
  final bool? Function(String sitting) flagOnDisk;
  RemoteExam? reply;
  String? carried;
  bool closeAnswered = true;
  bool expired = false;

  @override
  Future<RemoteExam?> examGet() async {
    log.add('examGet');
    if (expired) throw const PairingExpired();
    return reply;
  }

  @override
  Future<String?> examCarried(String sitting) async {
    log.add('carried $sitting');
    if (expired) throw const PairingExpired();
    return carried;
  }

  @override
  Future<bool> examClose(String sitting) async {
    log.add('close $sitting flag:${flagOnDisk(sitting)}');
    if (expired) throw const PairingExpired();
    return closeAnswered;
  }
}

class _HeldStore {
  _HeldStore(this.log, this.flagOnDisk);

  final List<String> log;
  final bool? Function(String sitting) flagOnDisk;
  final Map<(String, String), (String, String)> files = {};
  bool unreadable = false;
  bool unwritable = false;

  String? heldSitting(String root, String deckId) {
    log.add('held $root/$deckId');
    if (unreadable) throw const FileSystemException('the held file cannot be read');
    return files[(root, deckId)]?.$1;
  }

  void receive(StartedSitting record, String body) {
    log.add('receive ${record.sitting} flag:${flagOnDisk(record.sitting)}');
    if (unwritable) throw const FileSystemException('the held file cannot be written');
    files[(record.root, record.deckId)] = (record.sitting, body);
  }
}

void main() {
  late Directory support;
  late List<String> log;
  late StartedSittings sittings;
  late _Desk desk;
  late _HeldStore held;

  bool? flagOnDisk(String sitting) {
    final file = File('${support.path}/exam-sittings.json');
    if (!file.existsSync()) return null;
    final json = jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
    for (final record in json['sittings'] as List) {
      final map = record as Map<String, dynamic>;
      if (map['sitting'] == sitting) return map['fetched'] as bool;
    }
    return null;
  }

  ExamHandoff handoff({ServerClient? client}) => ExamHandoff(
    sittings: sittings,
    client: client ?? desk,
    heldSitting: held.heldSitting,
    receive: held.receive,
  );

  void restart() {
    StartedSittings.forgetLoaded();
    sittings = StartedSittings.load(support);
  }

  setUp(() {
    StartedSittings.forgetLoaded();
    support = Directory.systemTemp.createTempSync('exam-handoff-test');
    addTearDown(() => support.deleteSync(recursive: true));
    log = [];
    sittings = StartedSittings.load(support);
    desk = _Desk(log, flagOnDisk);
    held = _HeldStore(log, flagOnDisk);
  });

  test('an unfetched sitting with no held file is fetched, written, flagged, then closed, in that order', () async {
    sittings.put(_sitting());
    desk.carried = '{"exam":"one"}';

    final result = await handoff().onReady('sitting-1');

    expect(result, Handoff.fetched, reason: 'onReady(sitting-1): result $result, expected fetched');
    expect(
      log,
      ['held root-a/deck-1', 'carried sitting-1', 'receive sitting-1 flag:false', 'close sitting-1 flag:true'],
      reason: 'onReady(sitting-1): the held check, the fetch, the write before the flag, the close after the flag; got $log',
    );
    expect(
      held.files[('root-a', 'deck-1')],
      ('sitting-1', '{"exam":"one"}'),
      reason: 'after onReady: the held file under root-a/deck-1 holds the carried body verbatim',
    );
    restart();
    expect(sittings.forDeck('root-a', 'deck-1')?.fetched, isTrue, reason: 'after a restart: the record of sitting-1 reads fetched');
  });

  test('a fetched sitting is only closed', () async {
    sittings.put(_sitting(fetched: true));
    held.files[('root-a', 'deck-1')] = ('sitting-1', 'held body');
    desk.carried = 'another body';
    desk.closeAnswered = false;

    final first = await handoff().onReady('sitting-1');

    expect(first, Handoff.closed, reason: 'first poll, close unanswered: result $first, expected closed');
    expect(log, ['close sitting-1 flag:true'], reason: 'first poll: only the close is sent; got $log');

    log.clear();
    desk.closeAnswered = true;
    final second = await handoff().onReady('sitting-1');

    expect(second, Handoff.closed, reason: 'second poll: result $second, expected closed');
    expect(log, ['close sitting-1 flag:true'], reason: 'second poll: the close is sent again and nothing else; got $log');
    expect(
      held.files[('root-a', 'deck-1')],
      ('sitting-1', 'held body'),
      reason: 'after both polls: the held file of root-a/deck-1 is the one held before',
    );
  });

  test('a kill after the file and before the flag: the next poll writes nothing, sets the flag and closes', () async {
    sittings.put(_sitting());
    held.files[('root-a', 'deck-1')] = ('sitting-1', 'held body');
    desk.carried = 'another body';
    restart();

    final result = await handoff().onReady('sitting-1');

    expect(result, Handoff.closed, reason: 'poll after the kill: result $result, expected closed');
    expect(
      log,
      ['held root-a/deck-1', 'close sitting-1 flag:true'],
      reason: 'poll after the kill: the held check, then the close after the flag is on disk; got $log',
    );
    expect(
      held.files[('root-a', 'deck-1')],
      ('sitting-1', 'held body'),
      reason: 'poll after the kill: the held file of root-a/deck-1 is unchanged',
    );
  });

  test('a sitting this phone did not start causes no call', () async {
    final none = await handoff().onReady('sitting-1');

    expect(none, Handoff.none, reason: 'no records, ready(sitting-1): result $none, expected none');
    expect(log, isEmpty, reason: 'no records, ready(sitting-1): no call; got $log');

    sittings.put(_sitting());
    final other = await handoff().onReady('sitting-elsewhere');

    expect(other, Handoff.none, reason: 'a record of sitting-1, ready(sitting-elsewhere): result $other, expected none');
    expect(log, isEmpty, reason: 'a record of sitting-1, ready(sitting-elsewhere): no call; got $log');

    sittings.put(_sitting(deckId: 'deck-3', sitting: 'sitting-3'));
    held.files[('root-a', 'deck-3')] = ('sitting-older', 'held body');
    desk.carried = 'exam three';
    final heldOther = await handoff().onReady('sitting-3');

    expect(heldOther, Handoff.none, reason: 'deck-3 holds sitting-older, ready(sitting-3): result $heldOther, expected none');
    expect(log, ['held root-a/deck-3'], reason: 'deck-3 holds sitting-older, ready(sitting-3): the held check only, no desk call; got $log');
    expect(
      held.files[('root-a', 'deck-3')],
      ('sitting-older', 'held body'),
      reason: 'deck-3 holds sitting-older, ready(sitting-3): the held file is not overwritten',
    );
  });

  test('an exam finished before any poll is not fetched again, and nothing is written', () async {
    sittings.put(_sitting());
    held.files[('root-a', 'deck-1')] = ('sitting-1', 'held body');
    restart();
    sittings.forDeck('root-a', 'deck-1')!.fetched = true;
    sittings.save();
    held.files.remove(('root-a', 'deck-1'));
    restart();
    desk.carried = 'the same exam again';

    final result = await handoff().onReady('sitting-1');

    expect(result, Handoff.closed, reason: 'poll after the finish: result $result, expected closed');
    expect(log, ['close sitting-1 flag:true'], reason: 'poll after the finish: receive is not called, only the close is sent; got $log');
    expect(held.files, isEmpty, reason: 'poll after the finish: no held file is written; got ${held.files}');
  });

  test('a fetch that fails changes nothing and the next poll fetches', () async {
    sittings.put(_sitting());

    final failed = await handoff().onReady('sitting-1');

    expect(failed, Handoff.failed, reason: 'carried without a reply: result $failed, expected failed');
    expect(log, ['held root-a/deck-1', 'carried sitting-1'], reason: 'carried without a reply: no write and no close; got $log');
    expect(flagOnDisk('sitting-1'), isFalse, reason: 'carried without a reply: the record stays unfetched on disk');
    expect(held.files, isEmpty, reason: 'carried without a reply: no held file');

    log.clear();
    desk.carried = 'the exam';
    final fetched = await handoff().onReady('sitting-1');

    expect(fetched, Handoff.fetched, reason: 'next poll: result $fetched, expected fetched');
    expect(
      log,
      ['held root-a/deck-1', 'carried sitting-1', 'receive sitting-1 flag:false', 'close sitting-1 flag:true'],
      reason: 'next poll: the whole hand-off in order; got $log',
    );
  });

  test('a held file that cannot be written sends no close and leaves the record unfetched', () async {
    sittings.put(_sitting());
    desk.carried = 'the exam';
    held.unwritable = true;

    final result = await handoff().onReady('sitting-1');

    expect(result, Handoff.failed, reason: 'receive throws: result $result, expected failed');
    expect(
      log,
      ['held root-a/deck-1', 'carried sitting-1', 'receive sitting-1 flag:false'],
      reason: 'receive throws: no close after the failed write; got $log',
    );
    expect(flagOnDisk('sitting-1'), isFalse, reason: 'receive throws: the record stays unfetched on disk');
    restart();
    expect(sittings.forDeck('root-a', 'deck-1')?.fetched, isFalse, reason: 'receive throws, after a restart: the record reads unfetched');
  });

  test('a document the phone cannot read sends no close, and the claim before a start is blocked', () async {
    sittings.put(_sitting());
    desk.carried = 'the exam';
    desk.reply = _ready('sitting-1');
    held.unreadable = true;

    final result = await handoff().onReady('sitting-1');

    expect(result, Handoff.failed, reason: 'heldSitting throws: result $result, expected failed');
    expect(log, ['held root-a/deck-1'], reason: 'heldSitting throws: no fetch, no write, no close; got $log');

    log.clear();
    final claim = await handoff().claimBeforeStart();

    expect(claim, isA<ClaimBlocked>(), reason: 'claim with an unreadable held file: got $claim, expected ClaimBlocked');
    expect((claim as ClaimBlocked).record.sitting, 'sitting-1', reason: 'claim blocked: the record names sitting-1');
    expect(log, ['examGet', 'held root-a/deck-1'], reason: 'claim with an unreadable held file: the poll and the held check only; got $log');
  });

  test('the claim fetches a ready exam first; when that fails it blocks until the exam is discarded', () async {
    sittings.put(_sitting());
    sittings.put(_sitting(deckId: 'deck-2', sitting: 'sitting-2'));
    desk.reply = _ready('sitting-1');
    desk.carried = 'exam one';

    final fetched = await handoff().claimBeforeStart();

    expect(fetched, isA<ClaimClear>(), reason: 'claim over ready(sitting-1): got $fetched, expected ClaimClear');
    expect(
      log,
      ['examGet', 'held root-a/deck-1', 'carried sitting-1', 'receive sitting-1 flag:false', 'close sitting-1 flag:true'],
      reason: 'claim over ready(sitting-1): the poll, then the whole hand-off before the claim returns; got $log',
    );

    log.clear();
    desk.reply = _ready('sitting-2');
    desk.carried = null;
    final blocked = await handoff().claimBeforeStart();

    expect(blocked, isA<ClaimBlocked>(), reason: 'claim over ready(sitting-2), fetch fails: got $blocked, expected ClaimBlocked');
    final record = (blocked as ClaimBlocked).record;
    expect(record.sitting, 'sitting-2', reason: 'claim blocked: the record names sitting-2');
    expect(log, ['examGet', 'held root-a/deck-2', 'carried sitting-2'], reason: 'claim blocked: no close; got $log');

    log.clear();
    final again = await handoff().claimBeforeStart();
    expect(again, isA<ClaimBlocked>(), reason: 'second claim before the discard: got $again, expected ClaimBlocked');

    log.clear();
    final expiredDesk = _Desk(log, flagOnDisk)..expired = true;
    await expectLater(
      handoff(client: expiredDesk).discardReady(record),
      throwsA(isA<PairingExpired>()),
      reason: 'discard on an expired pairing: PairingExpired reaches the caller',
    );
    expect(flagOnDisk('sitting-2'), isFalse, reason: 'discard on an expired pairing: the record of sitting-2 stays on disk');

    log.clear();
    desk.closeAnswered = false;
    await handoff().discardReady(record);

    expect(log, ['close sitting-2 flag:false'], reason: 'discard: the close is sent while the record is still on disk; got $log');
    expect(flagOnDisk('sitting-2'), isNull, reason: 'discard with the close unanswered: the record of sitting-2 is removed from disk');
    restart();
    expect(sittings.forDeck('root-a', 'deck-2'), isNull, reason: 'discard, after a restart: no record for root-a/deck-2');

    log.clear();
    desk.closeAnswered = true;
    final clear = await handoff().claimBeforeStart();

    expect(clear, isA<ClaimClear>(), reason: 'claim after the discard: got $clear, expected ClaimClear');
    expect(log, ['examGet'], reason: 'claim after the discard: the poll only; got $log');
  });

  test('with two pairings, a generation started on one is fetched after switching away and back', () async {
    sittings.put(_sitting());
    sittings.put(_sitting(root: 'root-b', sitting: 'sitting-b'));
    held.files[('root-b', 'deck-1')] = ('sitting-b', 'held on b');
    final deskB = _Desk(log, flagOnDisk)..reply = const RemoteExam(phase: 'idle', deck: '', thinking: false);

    final away = await handoff(client: deskB).claimBeforeStart();

    expect(away, isA<ClaimClear>(), reason: 'pairing b, desk idle: got $away, expected ClaimClear');
    expect(log, ['examGet'], reason: 'pairing b, desk idle: the poll only; got $log');

    log.clear();
    restart();
    desk.carried = 'exam on a';
    final back = await handoff().onReady('sitting-1');

    expect(back, Handoff.fetched, reason: 'back on pairing a, ready(sitting-1): result $back, expected fetched');
    expect(
      log,
      ['held root-a/deck-1', 'carried sitting-1', 'receive sitting-1 flag:false', 'close sitting-1 flag:true'],
      reason: 'back on pairing a: the held check under root-a/deck-1, then the hand-off; got $log',
    );
    expect(held.files[('root-a', 'deck-1')], ('sitting-1', 'exam on a'), reason: 'back on pairing a: written under root-a/deck-1');
    expect(held.files[('root-b', 'deck-1')], ('sitting-b', 'held on b'), reason: 'back on pairing a: the file of root-b/deck-1 is untouched');
  });

  test('a pre-start poll that gets no reply is unreachable, with no fetch and no close', () async {
    sittings.put(_sitting());
    desk.carried = 'the exam';

    final claim = await handoff().claimBeforeStart();

    expect(claim, isA<ClaimUnreachable>(), reason: 'poll without a reply: got $claim, expected ClaimUnreachable');
    expect(log, ['examGet'], reason: 'poll without a reply: no other call; got $log');
  });

  test('the claim closes a ready exam that is already fetched, and one that is already held, then is clear', () async {
    sittings.put(_sitting(fetched: true));
    sittings.put(_sitting(deckId: 'deck-2', sitting: 'sitting-2'));
    held.files[('root-a', 'deck-2')] = ('sitting-2', 'held body');
    desk.carried = 'another body';
    desk.reply = _ready('sitting-1');

    final fetched = await handoff().claimBeforeStart();

    expect(fetched, isA<ClaimClear>(), reason: 'claim over fetched sitting-1: got $fetched, expected ClaimClear');
    expect(log, ['examGet', 'close sitting-1 flag:true'], reason: 'claim over fetched sitting-1: the close sent before the claim returned; got $log');

    log.clear();
    desk.reply = _ready('sitting-2');
    final heldClaim = await handoff().claimBeforeStart();

    expect(heldClaim, isA<ClaimClear>(), reason: 'claim over held sitting-2: got $heldClaim, expected ClaimClear');
    expect(
      log,
      ['examGet', 'held root-a/deck-2', 'close sitting-2 flag:true'],
      reason: 'claim over held sitting-2: the flag on disk, then the close before the claim returned; got $log',
    );
    expect(held.files[('root-a', 'deck-2')], ('sitting-2', 'held body'), reason: 'claim over held sitting-2: the held file unchanged');
  });

  test('a claim on an expired pairing throws PairingExpired and sends nothing else', () async {
    sittings.put(_sitting());
    desk.reply = _ready('sitting-1');
    desk.carried = 'the exam';
    desk.expired = true;

    await expectLater(
      handoff().claimBeforeStart(),
      throwsA(isA<PairingExpired>()),
      reason: 'claim on an expired pairing: PairingExpired reaches the caller',
    );
    expect(log, ['examGet'], reason: 'claim on an expired pairing: the poll only; got $log');
  });
}
