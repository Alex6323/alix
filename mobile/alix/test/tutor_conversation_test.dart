// Laws for the tutor conversation's use of the desktop's one remote ask
// slot: at most one GET in flight per conversation, and one POST holder per
// desktop across conversations. Fake client, no widget tree; `testWidgets`
// only for its fake-async zone, so `tester.pump` steps the poll timer.
import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/tutor_conversation.dart';

import 'support/fake_server_client.dart';

const _pollInterval = Duration(milliseconds: 10);

TutorCardContext _card(String id) => TutorCardContext(
      deckId: 'deck-rust',
      cardId: id,
      subject: 'Rust',
      front: 'front of $id',
      back: const ['back'],
    );

void main() {
  TutorConversation conversation(
    FakeServerClient client,
    TutorSlot slot,
    String cardId, {
    void Function(List<String> notes)? onNote,
  }) {
    final built = TutorConversation(
      card: _card(cardId),
      client: client,
      slot: slot,
      mint: (front, back) async => 'card-1',
      onNote: onNote ?? (_) {},
      onMessage: (_) {},
      pollInterval: _pollInterval,
    );
    addTearDown(built.dispose);
    return built;
  }

  testWidgets('a poll still in flight is never joined by a second GET', (tester) async {
    // The desktop returns its one terminal DTO on every GET; two overlapping
    // GETs would each apply it (a note appended twice). The first GET after
    // the note POST is parked past three poll intervals.
    final gate = Completer<RemoteAsk?>();
    final client = FakeServerClient(
      getAskReplies: const [RemoteAsk(thinking: false, answer: 'an answer')],
      getAskGates: {1: gate},
    );
    var noteCalls = 0;
    final slot = TutorSlot();
    final subject = conversation(client, slot, 'card-a', onNote: (_) => noteCalls++);

    await subject.send('q');
    await tester.pump(_pollInterval);
    expect(subject.transcript, hasLength(1));

    await subject.makeNote();
    await tester.pump(_pollInterval);
    expect(client.getAskCalls, 2, reason: 'the note poll made its first GET');
    await tester.pump(_pollInterval * 3);
    expect(client.getAskCalls, 2,
        reason: 'no tick fires while a GET is in flight');

    gate.complete(const RemoteAsk(thinking: false, note: ['one point']));
    await tester.pump();
    expect(noteCalls, 1);
    expect(subject.notePending, isFalse);
    expect(slot.owner, isNull, reason: 'a settled call gives the slot back');
  });

  testWidgets('one holder of the desktop slot at a time: a new card waits for the old one',
      (tester) async {
    // Card A's note is in flight (its GET parked). Card B, sharing the slot,
    // may neither send nor distill: the desktop would replace A's settled
    // job with B's on the next POST, and A would then poll B's result.
    final gate = Completer<RemoteAsk?>();
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, answer: 'answer a'),
        RemoteAsk(thinking: false, note: ['a']),
        RemoteAsk(thinking: false, answer: 'answer b'),
      ],
      getAskGates: {1: gate},
    );
    final slot = TutorSlot();
    List<String>? notedA;
    final a = conversation(client, slot, 'card-a', onNote: (notes) => notedA = notes);
    final b = conversation(client, slot, 'card-b');

    await a.send('question a');
    await tester.pump(_pollInterval);
    await a.makeNote();
    await tester.pump(_pollInterval);
    expect(slot.owner, same(a));

    expect(b.canSend, isFalse);
    expect(b.slotBusy, isTrue);
    await b.send('question b');
    expect(client.postAskHistories, hasLength(1),
        reason: "only A's question was posted; B's send was refused");
    expect(b.pendingQuestion, isNull);

    gate.complete(const RemoteAsk(thinking: false, note: ['a']));
    await tester.pump();
    expect(notedA, ['a'], reason: "A's note landed in A, not in B");
    expect(slot.owner, isNull);
    expect(b.canSend, isTrue);

    await b.send('question b');
    await tester.pump(_pollInterval);
    expect(b.transcript.single.a, 'answer b');
    expect(a.transcript, hasLength(1), reason: "B's answer did not land in A");
  });

  testWidgets("a settled reply without this call's outcome ends the call and frees the slot",
      (tester) async {
    // The DTO a desktop restarted under a pinned token serves on the next
    // GET: settled, everything null.
    const blank = RemoteAsk(thinking: false);
    final client = FakeServerClient(getAskReplies: const [
      RemoteAsk(thinking: false, answer: 'an answer'),
      blank,
    ]);
    final slot = TutorSlot();
    var noteCalls = 0;
    final subject = conversation(client, slot, 'card-a', onNote: (_) => noteCalls++);

    await subject.send('q');
    await tester.pump(_pollInterval);
    await subject.makeNote();
    await tester.pump(_pollInterval);
    expect(subject.notePending, isFalse, reason: 'a blank settled reply is a failed note');
    expect(slot.owner, isNull, reason: 'the failed note gives the slot back');
    expect(noteCalls, 0);
    expect(subject.message, 'The tutor call failed.');
    final callsAfterNote = client.getAskCalls;
    await tester.pump(_pollInterval * 3);
    expect(client.getAskCalls, callsAfterNote, reason: 'the note poll stopped');

    await subject.send('q2');
    await tester.pump(_pollInterval);
    expect(subject.pendingQuestion, isNull);
    expect(subject.transcript, hasLength(1), reason: 'no empty exchange is appended');
    expect(subject.takeRestoredQuestion(), 'q2', reason: 'the question goes back to the composer');
    expect(slot.owner, isNull);
  });

  testWidgets('a refused POST and an expired pairing give the slot back at once', (tester) async {
    final slot = TutorSlot();
    final refused = conversation(FakeServerClient(postAskReplies: const [false]), slot, 'card-a');
    await refused.send('q');
    expect(slot.owner, isNull);

    final expired = conversation(FakeServerClient(expireOnPostAsk: true), slot, 'card-b');
    await expired.send('q');
    expect(slot.owner, isNull);
    expect(expired.message, pairingExpiredMessage);
  });
}
