// Widget tests for the tutor sheet: a fake ServerClient (no network, no
// Rust dylib) and a fake mint callback drive the send/poll/draft/mint
// flows and the two error surfaces (unreachable, 401). Poll interval is
// shrunk well below the default so `tester.pump` can step through the
// fake's canned in-flight/settled replies without a long real wait (the
// binding runs each test inside a fake-async zone, so Timer.periodic only
// advances when pumped).
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/tutor_sheet.dart';

const _pollInterval = Duration(milliseconds: 10);

const _card = TutorCardContext(
  subject: 'Rust',
  front: 'Why does Rust use one owner per value?',
  back: ['so drops are deterministic'],
);

/// The tutor sheet's test double: canned replies queued per call, and the
/// sent history captured for the re-send assertion.
class FakeServerClient implements ServerClient {
  FakeServerClient({
    this.backend = 'Claude',
    List<bool>? postAskReplies,
    List<RemoteAsk>? getAskReplies,
    List<bool>? postDraftReplies,
    this.expireOnPostAsk = false,
    this.postAskGate,
  })  : postAskReplies = postAskReplies ?? const [true],
        getAskReplies = getAskReplies ?? const [],
        postDraftReplies = postDraftReplies ?? const [true];

  final String? backend;
  final List<bool> postAskReplies;
  final List<RemoteAsk> getAskReplies;
  final List<bool> postDraftReplies;
  final bool expireOnPostAsk;

  /// When set, postAsk parks on this until the test completes it: the
  /// "request still in flight while the sheet is dismissed" lever.
  final Completer<bool>? postAskGate;

  final List<List<TutorTurn>> postAskHistories = [];
  final List<List<TutorTurn>> postDraftHistories = [];
  int _askCall = 0;
  int _pollCall = 0;
  int _draftCall = 0;

  @override
  Future<String?> backendName() async => backend;

  @override
  Future<bool> postAsk(TutorCardContext card, List<TutorTurn> history, String question) async {
    postAskHistories.add(history);
    if (expireOnPostAsk) throw const PairingExpired();
    if (postAskGate != null) return postAskGate!.future;
    final reply = postAskReplies[_askCall.clamp(0, postAskReplies.length - 1)];
    _askCall++;
    return reply;
  }

  @override
  Future<RemoteAsk?> getAsk() async {
    final reply = getAskReplies[_pollCall.clamp(0, getAskReplies.length - 1)];
    _pollCall++;
    return reply;
  }

  @override
  Future<bool> postDraft(TutorCardContext card, List<TutorTurn> history) async {
    postDraftHistories.add(history);
    final reply = postDraftReplies[_draftCall.clamp(0, postDraftReplies.length - 1)];
    _draftCall++;
    return reply;
  }

  @override
  Future<String?> version() async => null;

  @override
  Future<bool> examStart(String deck) async => false;

  @override
  Future<RemoteExam?> examGet() async => null;

  @override
  Future<bool> examGrade(List<String> answers) async => false;

  @override
  Future<bool> examRemediate() async => false;

  @override
  Future<void> examClose() async {}

  @override
  void close() {}
}

void main() {
  Future<void> pumpSheet(
    WidgetTester tester, {
    required ServerClient client,
    Future<String> Function(String front, List<String> back)? mint,
  }) async {
    await tester.pumpWidget(MaterialApp(
      home: Scaffold(
        body: Builder(
          builder: (context) => Center(
            child: ElevatedButton(
              onPressed: () => showModalBottomSheet<void>(
                context: context,
                isScrollControlled: true,
                builder: (_) => TutorSheet(
                  card: _card,
                  client: client,
                  mint: mint ?? (front, back) async => 'card-1',
                  pollInterval: _pollInterval,
                ),
              ),
              child: const Text('open'),
            ),
          ),
        ),
      ),
    ));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
  }

  testWidgets('send: a pending working row names the backend, then the answer lands', (tester) async {
    final client = FakeServerClient(
      backend: 'Claude',
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

    await tester.enterText(find.byKey(const ValueKey('tutor-question-field')), 'first question');
    await tester.tap(find.byKey(const ValueKey('tutor-send-button')));
    await tester.pump();
    await tester.pump(_pollInterval);
    await tester.pumpAndSettle();
    expect(find.text('first answer'), findsOneWidget);

    await tester.enterText(find.byKey(const ValueKey('tutor-question-field')), 'second question');
    await tester.tap(find.byKey(const ValueKey('tutor-send-button')));
    await tester.pump();
    await tester.pump(_pollInterval);
    await tester.pumpAndSettle();

    expect(client.postAskHistories, hasLength(2));
    expect(client.postAskHistories[0], isEmpty);
    expect(client.postAskHistories[1], hasLength(1));
    expect(client.postAskHistories[1].single.q, 'first question');
    expect(client.postAskHistories[1].single.a, 'first answer');
  });

  testWidgets('unreachable: postAsk false shows the did-not-answer SnackBar and drops the pending row',
      (tester) async {
    final client = FakeServerClient(postAskReplies: const [false]);
    await pumpSheet(tester, client: client);

    await tester.enterText(find.byKey(const ValueKey('tutor-question-field')), 'anyone there?');
    await tester.tap(find.byKey(const ValueKey('tutor-send-button')));
    await tester.pumpAndSettle();

    expect(find.text('The desktop did not answer.'), findsOneWidget);
    expect(find.textContaining('is working'), findsNothing);
  });

  testWidgets('a 401 on send shows the exact re-pair SnackBar', (tester) async {
    final client = FakeServerClient(expireOnPostAsk: true);
    await pumpSheet(tester, client: client);

    await tester.enterText(find.byKey(const ValueKey('tutor-question-field')), 'anyone there?');
    await tester.tap(find.byKey(const ValueKey('tutor-send-button')));
    await tester.pumpAndSettle();

    expect(
      find.text('Pairing expired. Pair again from the deck list menu.'),
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

    await tester.enterText(find.byKey(const ValueKey('tutor-question-field')), 'first question');
    await tester.tap(find.byKey(const ValueKey('tutor-send-button')));
    await tester.pump();
    await tester.pump(_pollInterval);
    await tester.pumpAndSettle();
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
  });

  testWidgets('an empty transcript refuses "Make a card" locally, with no postDraft call', (tester) async {
    final client = FakeServerClient();
    await pumpSheet(tester, client: client);

    await tester.tap(find.byKey(const ValueKey('tutor-make-card-button')));
    await tester.pumpAndSettle();

    expect(find.text('Ask something first.'), findsOneWidget);
    expect(client.postDraftHistories, isEmpty);
  });

  testWidgets('dismissing the sheet mid-send starts no poll timer after dispose', (tester) async {
    // postAsk parks until the test releases it; the sheet is dismissed in
    // the meantime, so the resumed _send must not start a poll timer (a
    // leaked periodic timer fails the test at teardown, which is the net).
    final gate = Completer<bool>();
    final client = FakeServerClient(postAskGate: gate);
    await pumpSheet(tester, client: client);

    await tester.enterText(find.byKey(const ValueKey('tutor-question-field')), 'still out there?');
    await tester.tap(find.byKey(const ValueKey('tutor-send-button')));
    await tester.pump();

    tester.state<NavigatorState>(find.byType(Navigator)).pop();
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('tutor-question-field')), findsNothing,
        reason: 'the sheet must be gone before the reply settles');

    gate.complete(true);
    await tester.pump();
    await tester.pump(_pollInterval * 3);
    expect(tester.takeException(), isNull);
  });

  testWidgets('a settled error shows the failed SnackBar and restores the question', (tester) async {
    final client = FakeServerClient(
      getAskReplies: const [
        RemoteAsk(thinking: false, error: 'backend prose the user never sees'),
      ],
    );
    await pumpSheet(tester, client: client);

    await tester.enterText(find.byKey(const ValueKey('tutor-question-field')), 'my question');
    await tester.tap(find.byKey(const ValueKey('tutor-send-button')));
    await tester.pump();
    await tester.pump(_pollInterval);
    await tester.pumpAndSettle();

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
    await pumpSheet(tester, client: client);

    await tester.enterText(find.byKey(const ValueKey('tutor-question-field')), 'q');
    await tester.tap(find.byKey(const ValueKey('tutor-send-button')));
    await tester.pump();
    await tester.pump(_pollInterval);
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const ValueKey('tutor-make-card-button')));
    await tester.pump();
    await tester.pump(_pollInterval);

    final send = tester.widget<IconButton>(find.byKey(const ValueKey('tutor-send-button')));
    expect(send.onPressed, isNull);

    // Close the sheet so dispose cancels the still-thinking draft poll.
    tester.state<NavigatorState>(find.byType(Navigator)).pop();
    await tester.pumpAndSettle();
  });
}
