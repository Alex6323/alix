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
    Future<String> Function(String front, List<String> back)? mint,
    Future<String> Function(String block)? record,
  }) {
    final built = TutorConversation(
      card: _card(cardId),
      client: client,
      slot: slot,
      mint: mint ?? (front, back) async => 'card-1',
      record: record ?? (block) async => 'card-1',
      onNote: onNote ?? (_) {},
      onMessage: (_) {},
      pollInterval: _pollInterval,
    );
    addTearDown(built.dispose);
    return built;
  }

  /// Sends one question and polls its answer, then asks for a draft and
  /// polls it: the two getAsk replies a draft flow consumes.
  Future<void> drafted(WidgetTester tester, TutorConversation subject) async {
    await subject.send('q');
    await tester.pump(_pollInterval);
    expect(subject.transcript, hasLength(1));
    await subject.makeCard();
    await tester.pump(_pollInterval);
    expect(subject.draft, isNotNull, reason: 'the draft settled');
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

  testWidgets('a grounded draft is minted on the desktop and recorded under the returned block',
      (tester) async {
    const block = '## f edited\nb\n<!-- id: card-desk -->\n';
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, answer: 'an answer'),
        RemoteAsk(thinking: false, draft: DraftCard(front: 'f', back: ['b'])),
      ],
      postCardReplies: const [AddCardMinted(MintedCard(id: 'card-desk', block: block))],
    );
    var mintCalls = 0;
    final recorded = <String>[];
    final subject = conversation(
      client,
      TutorSlot(),
      'card-a',
      mint: (_, _) async {
        mintCalls++;
        return 'card-phone';
      },
      record: (block) async {
        recorded.add(block);
        return 'card-desk';
      },
    );
    await drafted(tester, subject);

    expect(await subject.confirmDraft('f edited', ['b']), isTrue);

    final call = client.postCardCalls.single;
    expect(call.deckId, 'deck-rust', reason: 'the create names the served deck');
    expect(call.front, 'f edited', reason: 'the edited front goes to the desktop');
    expect(call.back, ['b']);
    expect(recorded, [block], reason: 'the phone records the desktop block unchanged');
    expect(mintCalls, 0, reason: 'a grounded draft is never minted on the phone alone');
    expect(subject.message, 'Card added.');
    expect(subject.draft, isNull);
  });

  testWidgets('a draft the desktop does not keep is minted nowhere and stays open', (tester) async {
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, answer: 'an answer'),
        RemoteAsk(thinking: false, draft: DraftCard(front: 'f', back: ['b'])),
      ],
      postCardReplies: const [AddCardRefused(404)],
    );
    var mintCalls = 0;
    var recordCalls = 0;
    final subject = conversation(
      client,
      TutorSlot(),
      'card-a',
      mint: (_, _) async {
        mintCalls++;
        return 'card-phone';
      },
      record: (_) async {
        recordCalls++;
        return 'card-desk';
      },
    );
    await drafted(tester, subject);

    expect(await subject.confirmDraft('f', ['b']), isFalse);

    expect(client.postCardCalls, hasLength(1));
    expect(mintCalls, 0, reason: 'the phone never holds a card the desktop did not keep');
    expect(recordCalls, 0);
    expect(subject.message, 'The desktop did not keep the card.');
    expect(subject.draft, isNotNull, reason: 'the draft stays open for another try');
  });

  testWidgets('a refused Add unlocks the draft and names the refusal', (tester) async {
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, answer: 'an answer'),
        RemoteAsk(thinking: false, draft: DraftCard(front: 'f', back: ['b'])),
      ],
      postCardReplies: const [AddCardRefused(422)],
    );
    final subject = conversation(client, TutorSlot(), 'card-a');
    await drafted(tester, subject);

    expect(await subject.confirmDraft('f', ['b']), isFalse);

    expect(subject.message, 'The desktop refused the card: a duplicate, or malformed.');
    expect(subject.draftLocked, isFalse, reason: 'a refusal is edited, not retried as is');
    expect(subject.draft, isNotNull);
  });

  testWidgets('an unanswered Add keeps the draft locked and its retry records the kept card',
      (tester) async {
    const block = '## f\nb\n<!-- id: card-desk -->\n';
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, answer: 'an answer'),
        RemoteAsk(thinking: false, draft: DraftCard(front: 'f', back: ['b'])),
      ],
      postCardReplies: const [
        AddCardNoAnswer(),
        AddCardMinted(MintedCard(id: 'card-desk', block: block)),
      ],
    );
    final recorded = <String>[];
    final subject = conversation(
      client,
      TutorSlot(),
      'card-a',
      record: (block) async {
        recorded.add(block);
        return 'card-desk';
      },
    );
    await drafted(tester, subject);

    expect(await subject.confirmDraft('f', ['b']), isFalse);
    expect(
      subject.message,
      'No answer from the desktop. The card may already be added; tap Add to retry.',
    );
    expect(subject.draftLocked, isTrue, reason: 'the retry must send the same text');
    expect(recorded, isEmpty);

    expect(await subject.confirmDraft('f', ['b']), isTrue);
    expect(recorded, [block]);
    expect(subject.draftLocked, isFalse);
    expect(subject.draft, isNull);
  });

  testWidgets('an Add in flight keeps a retired conversation alive', (tester) async {
    const block = '## f\nb\n<!-- id: card-desk -->\n';
    final gate = Completer<AddCardReply>();
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, answer: 'an answer'),
        RemoteAsk(thinking: false, draft: DraftCard(front: 'f', back: ['b'])),
      ],
      postCardGate: gate,
    );
    final recorded = <String>[];
    final subject = conversation(
      client,
      TutorSlot(),
      'card-a',
      record: (block) async {
        recorded.add(block);
        return 'card-desk';
      },
    );
    await drafted(tester, subject);

    final outcome = subject.confirmDraft('f', ['b']);
    await tester.pump();
    final busyWhileAdding = subject.busy;
    gate.complete(const AddCardMinted(MintedCard(id: 'card-desk', block: block)));

    expect(await outcome, isTrue);
    expect(recorded, [block]);
    expect(
      busyWhileAdding,
      isTrue,
      reason: 'the review screen retires only conversations whose busy flag is true',
    );
  });

  testWidgets('a card-only Add locks the draft and rejects a second tap', (tester) async {
    final gate = Completer<String>();
    var mintCalls = 0;
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, answer: 'an answer', cardOnly: true),
        RemoteAsk(
          thinking: false,
          cardOnly: true,
          draft: DraftCard(front: 'f', back: ['b']),
        ),
      ],
    );
    final subject = conversation(
      client,
      TutorSlot(),
      'card-a',
      mint: (front, back) {
        mintCalls++;
        return gate.future;
      },
    );
    await drafted(tester, subject);

    final first = subject.confirmDraft('f', ['b']);
    await tester.pump();
    final lockedWhileAdding = subject.draftLocked;
    final second = subject.confirmDraft('f', ['b']);
    await tester.pump();
    gate.complete('card-phone');

    expect(await first, isTrue);
    expect(await second, isFalse, reason: 'one Add is accepted');
    expect(mintCalls, 1, reason: 'a second tap cannot mint the same local card twice');
    expect(lockedWhileAdding, isTrue);
  });

  testWidgets('a card-only draft is minted on the phone alone', (tester) async {
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, answer: 'an answer', cardOnly: true),
        RemoteAsk(thinking: false, cardOnly: true, draft: DraftCard(front: 'f', back: ['b'])),
      ],
    );
    String? mintedFront;
    var recordCalls = 0;
    final subject = conversation(
      client,
      TutorSlot(),
      'card-a',
      mint: (front, back) async {
        mintedFront = front;
        return 'card-phone';
      },
      record: (_) async {
        recordCalls++;
        return 'card-desk';
      },
    );
    await drafted(tester, subject);

    expect(await subject.confirmDraft('f', ['b']), isTrue);

    expect(client.postCardCalls, isEmpty, reason: 'the desktop does not serve this deck');
    expect(mintedFront, 'f');
    expect(recordCalls, 0);
    expect(subject.message, 'Card added.');
  });

  testWidgets('the unsent text belongs to the conversation: a send clears it, a refused send keeps it',
      (tester) async {
    final client = FakeServerClient(
      getAskReplies: const [RemoteAsk(thinking: false, answer: 'an answer')],
    );
    final slot = TutorSlot();
    final subject = conversation(client, slot, 'card-a');

    subject.unsentQuestion = '   ';
    await subject.send('   ');
    expect(subject.unsentQuestion, '   ',
        reason: 'a blank send is refused and changes nothing');

    subject.unsentQuestion = 'why one owner';
    await subject.send('why one owner');
    expect(subject.unsentQuestion, '', reason: 'the accepted send consumed the text');
    await tester.pump(_pollInterval);
    expect(subject.transcript.single.q, 'why one owner');
  });
}
