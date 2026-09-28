// Widget tests for the tutor sheet over its conversation: a fake
// ServerClient (no network, no Rust dylib) and a fake mint callback drive the
// send/poll/draft/mint/note flows and the two error surfaces (unreachable,
// 401). Poll interval is shrunk well below the default so `tester.pump` can
// step through the fake's canned in-flight/settled replies without a long
// real wait (the binding runs each test inside a fake-async zone, so
// Timer.periodic only advances when pumped).
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/tutor_conversation.dart';
import 'package:alix_mobile/tutor_sheet.dart';

import 'support/fake_server_client.dart';

const _pollInterval = Duration(milliseconds: 10);

const _card = TutorCardContext(
  deckId: 'deck-rust',
  cardId: 'card-ownership',
  subject: 'Rust',
  front: 'Why does Rust use one owner per value?',
  back: ['so drops are deterministic'],
);

/// The conversation under test plus the messages it announced to its owner.
class _Harness {
  _Harness(this.conversation);

  final TutorConversation conversation;
  final List<String> messages = [];
  bool disposed = false;

  /// Cancels a poll that is still running when a test ends with a call in
  /// flight (a leaked periodic timer fails the test at teardown).
  void dispose() {
    if (disposed) return;
    disposed = true;
    conversation.dispose();
  }
}

void main() {
  Future<_Harness> pumpSheet(
    WidgetTester tester, {
    required ServerClient client,
    Future<String> Function(String front, List<String> back)? mint,
    void Function(List<String> notes)? onNote,
    Duration pollInterval = _pollInterval,
  }) async {
    late final _Harness harness;
    harness = _Harness(TutorConversation(
      card: _card,
      client: client,
      slot: TutorSlot(),
      mint: mint ?? (front, back) async => 'card-1',
      onNote: onNote ?? (_) {},
      onMessage: (text) => harness.messages.add(text),
      pollInterval: pollInterval,
    ));
    addTearDown(harness.dispose);
    await tester.pumpWidget(MaterialApp(
      theme: alixDark(),
      home: Scaffold(
        body: Builder(
          builder: (context) => Center(
            child: ElevatedButton(
              onPressed: () => showModalBottomSheet<void>(
                context: context,
                isScrollControlled: true,
                builder: (_) => TutorSheet(conversation: harness.conversation),
              ),
              child: const Text('open'),
            ),
          ),
        ),
      ),
    ));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    return harness;
  }

  Future<void> ask(WidgetTester tester, String question) async {
    await tester.enterText(find.byKey(const ValueKey('tutor-question-field')), question);
    await tester.tap(find.byKey(const ValueKey('tutor-send-button')));
    await tester.pump();
    await tester.pump(_pollInterval);
    await tester.pumpAndSettle();
  }

  Future<void> closeSheet(WidgetTester tester) async {
    tester.state<NavigatorState>(find.byType(Navigator)).pop();
    await tester.pumpAndSettle();
  }

  Future<void> reopenSheet(WidgetTester tester) async {
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
  }

  VoidCallback? onPressedOf(WidgetTester tester, String key) =>
      tester.widget<TextButton>(find.byKey(ValueKey(key))).onPressed;

  testWidgets('send: a pending working row names the backend, then the answer lands', (tester) async {
    final client = FakeServerClient(
      backendReply: 'Claude',
      getAskReplies: const [
        RemoteAsk(thinking: true, elapsed: 1),
        RemoteAsk(thinking: false, answer: 'so drops are deterministic'),
      ],
    );
    await pumpSheet(tester, client: client);

    await tester.enterText(find.byKey(const ValueKey('tutor-question-field')), 'why one owner?');
    await tester.tap(find.byKey(const ValueKey('tutor-send-button')));
    await tester.pump();

    await tester.pump(_pollInterval);
    expect(find.textContaining('Claude is working'), findsOneWidget);

    await tester.pump(_pollInterval);
    await tester.pumpAndSettle();
    expect(find.text('so drops are deterministic'), findsOneWidget);
    expect(find.textContaining('Claude is working'), findsNothing);
  });

  testWidgets('a second send re-sends the first turn verbatim as history', (tester) async {
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, answer: 'first answer'),
        RemoteAsk(thinking: false, answer: 'second answer'),
      ],
    );
    await pumpSheet(tester, client: client);

    await ask(tester, 'first question');
    expect(find.text('first answer'), findsOneWidget);
    await ask(tester, 'second question');

    expect(client.postAskHistories, hasLength(2));
    expect(client.postAskHistories[0], isEmpty);
    expect(client.postAskHistories[1], hasLength(1));
    expect(client.postAskHistories[1].single.q, 'first question');
    expect(client.postAskHistories[1].single.a, 'first answer');
  });

  testWidgets('a reply with units renders through the card unit widgets, not as raw text',
      (tester) async {
    final client = FakeServerClient(
      getAskReplies: [
        RemoteAsk(
          thinking: false,
          answer: 'raw answer text\n```\nfn main() {}\n```',
          units: [
            ReviewSentenceModel(
              text: 'A fenced block:',
              runs: const [
                InlineRunModel(text: 'A fenced ', bold: false, italic: false, code: false),
                InlineRunModel(text: 'block', bold: true, italic: false, code: false),
                InlineRunModel(text: ':', bold: false, italic: false, code: false),
              ],
            ),
            ReviewCodeModel(const ['fn main() {}']),
          ],
        ),
      ],
    );
    await pumpSheet(tester, client: client);

    await ask(tester, 'show me code');

    expect(find.text('A fenced block:', findRichText: true), findsOneWidget,
        reason: 'a sentence renders its runs (here three, one bold) as one rich text');
    expect(find.text('fn main() {}'), findsOneWidget);
    expect(find.textContaining('raw answer text'), findsNothing,
        reason: 'the raw answer is history for the next turn, never shown beside its units');
  });

  testWidgets('the card-only line sits under the latest card-only reply only', (tester) async {
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, answer: 'from the card', cardOnly: true),
        RemoteAsk(thinking: false, answer: 'grounded', status: 'The tutor has partial context.'),
      ],
    );
    await pumpSheet(tester, client: client);

    await ask(tester, 'first');
    expect(find.byKey(const ValueKey('tutor-card-only-line')), findsOneWidget);
    expect(find.text(cardOnlyMessage), findsOneWidget);

    await ask(tester, 'second');
    expect(find.text('grounded'), findsOneWidget);
    expect(find.byKey(const ValueKey('tutor-card-only-line')), findsNothing,
        reason: 'a grounded reply after a card-only one removes the line');
    expect(find.text('The tutor has partial context.'), findsOneWidget,
        reason: "the desktop's status line for the latest reply shows");
  });

  testWidgets('unreachable: postAsk false shows the did-not-answer line and drops the pending row',
      (tester) async {
    final client = FakeServerClient(postAskReplies: const [false]);
    await pumpSheet(tester, client: client);

    await tester.enterText(find.byKey(const ValueKey('tutor-question-field')), 'anyone there?');
    await tester.tap(find.byKey(const ValueKey('tutor-send-button')));
    await tester.pumpAndSettle();

    expect(find.text('The desktop did not answer.'), findsOneWidget);
    expect(find.textContaining('is working'), findsNothing);
  });

  testWidgets('a 401 on send shows the exact re-pair line', (tester) async {
    final client = FakeServerClient(expireOnPostAsk: true);
    await pumpSheet(tester, client: client);

    await tester.enterText(find.byKey(const ValueKey('tutor-question-field')), 'anyone there?');
    await tester.tap(find.byKey(const ValueKey('tutor-send-button')));
    await tester.pumpAndSettle();

    expect(
      find.text('Pairing expired. Pair again from Settings → Connected devices.'),
      findsOneWidget,
    );
  });

  testWidgets('draft -> edit -> mint: the mint callback gets the edited front and the drafted back',
      (tester) async {
    // Ask and draft share the one poll endpoint (the server's single ask
    // slot): the ask settles on the first getAsk() call, the draft on the
    // second.
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, answer: 'first answer'),
        RemoteAsk(
          thinking: false,
          draft: DraftCard(front: 'Why one owner per value?', back: ['so drops are deterministic']),
        ),
      ],
    );
    String? mintedFront;
    List<String>? mintedBack;
    await pumpSheet(
      tester,
      client: client,
      mint: (front, back) async {
        mintedFront = front;
        mintedBack = back;
        return 'card-1';
      },
    );

    await ask(tester, 'first question');
    expect(find.text('first answer'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('tutor-make-card-button')));
    await tester.pump();
    await tester.pump(_pollInterval);
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('tutor-draft-front-field')), findsOneWidget);
    await tester.enterText(
      find.byKey(const ValueKey('tutor-draft-front-field')),
      'Why exactly one owner per value?',
    );
    await tester.tap(find.byKey(const ValueKey('tutor-draft-confirm-button')));
    await tester.pumpAndSettle();

    expect(mintedFront, 'Why exactly one owner per value?');
    expect(mintedBack, ['so drops are deterministic']);
    expect(find.text('Card added.'), findsOneWidget);
    expect(client.postDraftHistories.single, hasLength(1),
        reason: 'a draft is made from the whole conversation');
  });

  testWidgets('"Make a card" and "Make a note" are disabled until an answer exists', (tester) async {
    final client = FakeServerClient(
      getAskReplies: const [RemoteAsk(thinking: false, answer: 'an answer')],
    );
    await pumpSheet(tester, client: client);

    expect(onPressedOf(tester, 'tutor-make-card-button'), isNull);
    expect(onPressedOf(tester, 'tutor-make-note-button'), isNull);
    await tester.tap(find.byKey(const ValueKey('tutor-make-card-button')));
    await tester.tap(find.byKey(const ValueKey('tutor-make-note-button')));
    await tester.pumpAndSettle();
    expect(client.postDraftHistories, isEmpty);
    expect(client.postNoteHistories, isEmpty);

    await ask(tester, 'a question');
    expect(onPressedOf(tester, 'tutor-make-card-button'), isNotNull);
    expect(onPressedOf(tester, 'tutor-make-note-button'), isNotNull);
  });

  testWidgets('dismissing the sheet mid-send loses nothing: the answer is there on reopen',
      (tester) async {
    // postAsk parks until the test releases it; the sheet is dismissed in
    // the meantime. The conversation outlives the sheet, so the reply still
    // lands and the reopened sheet shows it.
    final gate = Completer<bool>();
    final client = FakeServerClient(
      postAskGate: gate,
      getAskReplies: const [RemoteAsk(thinking: false, answer: 'still here')],
    );
    final harness = await pumpSheet(tester, client: client);

    await tester.enterText(find.byKey(const ValueKey('tutor-question-field')), 'still out there?');
    await tester.tap(find.byKey(const ValueKey('tutor-send-button')));
    await tester.pump();

    await closeSheet(tester);
    expect(find.byKey(const ValueKey('tutor-question-field')), findsNothing,
        reason: 'the sheet must be gone before the reply settles');

    gate.complete(true);
    await tester.pump();
    await tester.pump(_pollInterval);
    await tester.pump();
    expect(harness.conversation.transcript.single.a, 'still here');

    await reopenSheet(tester);
    expect(find.text('still out there?'), findsOneWidget);
    expect(find.text('still here'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('a settled error shows the failed line and restores the question', (tester) async {
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, error: 'backend prose the user never sees'),
      ],
    );
    await pumpSheet(tester, client: client);

    await ask(tester, 'my question');

    expect(find.text('The tutor call failed.'), findsOneWidget);
    final field = tester.widget<TextField>(find.byKey(const ValueKey('tutor-question-field')));
    expect(field.controller?.text, 'my question',
        reason: 'the unanswered question goes back in the input, nothing is lost');
    expect(find.textContaining('backend prose'), findsNothing,
        reason: 'the DTO error is backend prose, never shown raw');
  });

  testWidgets('send stays disabled while a draft is in flight', (tester) async {
    // The ask settles on the first getAsk reply; the draft poll then sees
    // thinking forever. A send tap during the draft would cancel its poll
    // timer and orphan the working row, so the button must be disabled.
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, answer: 'an answer'),
        RemoteAsk(thinking: true, elapsed: 1),
      ],
    );
    final harness = await pumpSheet(tester, client: client);

    await ask(tester, 'q');

    await tester.tap(find.byKey(const ValueKey('tutor-make-card-button')));
    await tester.pump();
    await tester.pump(_pollInterval);

    final send = tester.widget<IconButton>(find.byKey(const ValueKey('tutor-send-button')));
    expect(send.onPressed, isNull);

    // The still-thinking draft poll belongs to the conversation, not the
    // sheet: closing the sheet leaves it running, disposing the conversation
    // cancels it.
    await closeSheet(tester);
    harness.dispose();
  });

  testWidgets('note -> lines: onNote gets the lines and the "note saved" line shows', (tester) async {
    // Ask and note share the one poll endpoint (the server's single ask
    // slot): the ask settles on the first getAsk() call, the note on the
    // second, exactly like the draft flow above.
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, answer: 'first answer'),
        RemoteAsk(thinking: false, note: ['a', 'b']),
      ],
    );
    List<String>? notedLines;
    await pumpSheet(
      tester,
      client: client,
      onNote: (notes) => notedLines = notes,
    );

    await ask(tester, 'first question');
    expect(find.text('first answer'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('tutor-make-note-button')));
    await tester.pump();
    await tester.pump(_pollInterval);
    await tester.pumpAndSettle();

    expect(notedLines, ['a', 'b']);
    expect(find.text('note saved'), findsOneWidget);
    expect(client.postNoteHistories, hasLength(1));
    expect(client.postNoteHistories.single.single.q, 'first question');
    expect(onPressedOf(tester, 'tutor-make-note-button'), isNull,
        reason: 'everything said so far is noted: nothing is left to distill');
  });

  testWidgets('the note watermark: a second note sends only the exchanges since the first',
      (tester) async {
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, answer: 'first answer'),
        RemoteAsk(thinking: false, note: ['a']),
        RemoteAsk(thinking: false, answer: 'second answer'),
        RemoteAsk(thinking: false, note: ['b']),
      ],
    );
    await pumpSheet(tester, client: client);

    await ask(tester, 'first question');
    await tester.tap(find.byKey(const ValueKey('tutor-make-note-button')));
    await tester.pump();
    await tester.pump(_pollInterval);
    await tester.pumpAndSettle();

    await ask(tester, 'second question');
    expect(onPressedOf(tester, 'tutor-make-note-button'), isNotNull,
        reason: 'a new answer after the note is there to distill');
    await tester.tap(find.byKey(const ValueKey('tutor-make-note-button')));
    await tester.pump();
    await tester.pump(_pollInterval);
    await tester.pumpAndSettle();

    expect(client.postNoteHistories, hasLength(2));
    expect(client.postNoteHistories[0].map((turn) => turn.q), ['first question']);
    expect(client.postNoteHistories[1].map((turn) => turn.q), ['second question'],
        reason: 'the first exchange was noted already; the desktop condenses the rest');
    expect(client.postAskHistories[1].map((turn) => turn.q), ['first question'],
        reason: 'a question still carries the whole conversation as history');
  });

  testWidgets('note -> []: the "nothing to save" line shows, onNote is NOT called, nothing is consumed',
      (tester) async {
    // The load-bearing three-state distinction from T4.1: an empty list is
    // itself a settled result, not "still pending" and not an error.
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, answer: 'first answer'),
        RemoteAsk(thinking: false, note: []),
      ],
    );
    var onNoteCalled = false;
    await pumpSheet(
      tester,
      client: client,
      onNote: (_) => onNoteCalled = true,
    );

    await ask(tester, 'first question');

    await tester.tap(find.byKey(const ValueKey('tutor-make-note-button')));
    await tester.pump();
    await tester.pump(_pollInterval);
    await tester.pumpAndSettle();

    expect(find.text('nothing to save'), findsOneWidget);
    expect(onNoteCalled, isFalse);
    expect(onPressedOf(tester, 'tutor-make-note-button'), isNotNull,
        reason: 'the desktop moves its watermark only on a saved note; so does the phone');
  });

  testWidgets('a settled error on "Make a note" shows the failed line, onNote NOT called', (tester) async {
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, answer: 'first answer'),
        RemoteAsk(thinking: false, error: 'backend prose the user never sees'),
      ],
    );
    var onNoteCalled = false;
    await pumpSheet(
      tester,
      client: client,
      onNote: (_) => onNoteCalled = true,
    );

    await ask(tester, 'first question');

    await tester.tap(find.byKey(const ValueKey('tutor-make-note-button')));
    await tester.pump();
    await tester.pump(_pollInterval);
    await tester.pumpAndSettle();

    expect(find.text('The tutor call failed.'), findsOneWidget);
    expect(onNoteCalled, isFalse);
  });

  testWidgets('closing the sheet while a note is pending: the note applies, and reopening shows it',
      (tester) async {
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, answer: 'first answer'),
        RemoteAsk(thinking: true, elapsed: 1),
        RemoteAsk(thinking: false, note: ['a']),
      ],
    );
    // A poll far slower than the sheet's close animation, so pumpAndSettle
    // on the close cannot be what lands the note.
    const slowPoll = Duration(seconds: 5);
    List<String>? notedLines;
    final harness = await pumpSheet(
      tester,
      client: client,
      onNote: (notes) => notedLines = notes,
      pollInterval: slowPoll,
    );

    await tester.enterText(find.byKey(const ValueKey('tutor-question-field')), 'first question');
    await tester.tap(find.byKey(const ValueKey('tutor-send-button')));
    await tester.pump();
    await tester.pump(slowPoll);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('tutor-make-note-button')));
    await tester.pump();
    await tester.pump(slowPoll);
    expect(find.textContaining('is working'), findsOneWidget);

    await closeSheet(tester);
    expect(find.byKey(const ValueKey('tutor-question-field')), findsNothing);
    expect(notedLines, isNull, reason: 'the note is still in flight when the sheet goes');

    await tester.pump(slowPoll);
    await tester.pump();
    expect(notedLines, ['a'], reason: 'the result applies with the sheet closed');
    expect(harness.messages, contains('note saved'),
        reason: 'the owner is told, so it can announce what the sheet cannot');

    await reopenSheet(tester);
    expect(find.text('first question'), findsOneWidget);
    expect(find.text('first answer'), findsOneWidget);
    expect(find.text('note saved'), findsOneWidget);
    expect(onPressedOf(tester, 'tutor-make-note-button'), isNull);
  });
}
