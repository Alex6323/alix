// Widget tests for the exam screen: a fake ServerClient (no network, no
// Rust dylib) drives examStart/examGet/examGrade/examRemediate/examClose,
// and recording fake callbacks stand in for the bridge's applyExamPassed/
// applyRemediation. Poll interval is shrunk well below the default so
// `tester.pump` can step through the fake's canned phase-walk without a
// long real wait (Timer.periodic only advances when pumped).
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/exam_screen.dart';
import 'package:alix_mobile/server_client.dart';

const _pollInterval = Duration(milliseconds: 10);

/// The exam screen's test double: canned replies queued per call, and the
/// sent answers captured for the order assertion.
class FakeServerClient implements ServerClient {
  FakeServerClient({
    this.startReply = true,
    this.expireOnStart = false,
    List<RemoteExam>? getReplies,
    this.expireOnGet = false,
    List<bool>? gradeReplies,
    this.expireOnGrade = false,
    List<bool>? remediateReplies,
    this.expireOnRemediate = false,
    Map<int, Completer<RemoteExam?>>? getGates,
  })  : getReplies = getReplies ?? const [],
        gradeReplies = gradeReplies ?? const [true],
        remediateReplies = remediateReplies ?? const [true],
        getGates = getGates ?? const {};

  final bool startReply;
  final bool expireOnStart;
  final List<RemoteExam> getReplies;
  final bool expireOnGet;
  final List<bool> gradeReplies;
  final bool expireOnGrade;
  final List<bool> remediateReplies;
  final bool expireOnRemediate;

  /// Parks a specific (0-based) `examGet()` call on a completer instead of
  /// answering immediately: the lever for putting two polls in flight at
  /// once (a periodic tick firing again before the prior call's own
  /// cancellation lands), the real-world race the exactly-once guard
  /// protects against.
  final Map<int, Completer<RemoteExam?>> getGates;

  String? startedDeck;
  final List<List<String>> gradeAnswers = [];
  int closeCalls = 0;
  int _getCall = 0;
  int _gradeCall = 0;
  int _remediateCall = 0;

  @override
  Future<bool> examStart(String deck) async {
    startedDeck = deck;
    if (expireOnStart) throw const PairingExpired();
    return startReply;
  }

  @override
  Future<RemoteExam?> examGet() async {
    if (expireOnGet) throw const PairingExpired();
    final call = _getCall;
    _getCall++;
    final gate = getGates[call];
    if (gate != null) return gate.future;
    if (getReplies.isEmpty) return null;
    return getReplies[call.clamp(0, getReplies.length - 1)];
  }

  @override
  Future<bool> examGrade(List<String> answers) async {
    gradeAnswers.add(answers);
    if (expireOnGrade) throw const PairingExpired();
    final reply = gradeReplies[_gradeCall.clamp(0, gradeReplies.length - 1)];
    _gradeCall++;
    return reply;
  }

  @override
  Future<bool> examRemediate() async {
    if (expireOnRemediate) throw const PairingExpired();
    final reply = remediateReplies[_remediateCall.clamp(0, remediateReplies.length - 1)];
    _remediateCall++;
    return reply;
  }

  @override
  Future<void> examClose() async => closeCalls++;

  @override
  Future<String?> version() async => null;

  @override
  Future<String?> backendName() async => null;

  @override
  Future<bool> postAsk(TutorCardContext card, List<TutorTurn> history, String question) async => false;

  @override
  Future<RemoteAsk?> getAsk() async => null;

  @override
  Future<bool> postDraft(TutorCardContext card, List<TutorTurn> history) async => false;

  @override
  Future<bool> postNote(TutorCardContext card, List<TutorTurn> history) async => false;

  @override
  Future<bool> generateStart(String url, {String? guidance}) async => false;

  @override
  Future<RemoteGenerate?> generateGet() async => null;

  @override
  Future<void> generateClose() async {}

  @override
  void close() {}
}

void main() {
  Future<void> pumpExam(
    WidgetTester tester, {
    required ServerClient client,
    List<BigInt> passedCalls = const [],
    List<(String, BigInt)> remediationCalls = const [],
    int remediationCountReply = 1,
  }) async {
    await tester.pumpWidget(MaterialApp(
      home: ExamScreen(
        deckName: 'rust.txt',
        client: client,
        applyPassed: (nowMs) => passedCalls.add(nowMs),
        applyRemediation: (cardsText, nowMs) {
          remediationCalls.add((cardsText, nowMs));
          return remediationCountReply;
        },
        nowMs: () => BigInt.from(1000),
        pollInterval: _pollInterval,
      ),
    ));
    await tester.pump();
  }

  const answering2 = RemoteExam(
    phase: 'answering',
    deck: 'rust.txt',
    strictness: 'balanced',
    questions: ['Why ownership?', 'What is borrowing?'],
    grades: [],
    gaps: [],
    canRemediate: false,
    isTrace: false,
    thinking: false,
  );

  const generating = RemoteExam(
    phase: 'generating',
    deck: 'rust.txt',
    strictness: 'balanced',
    questions: [],
    grades: [],
    gaps: [],
    canRemediate: false,
    isTrace: false,
    thinking: true,
    elapsed: 1,
  );

  const grading = RemoteExam(
    phase: 'grading',
    deck: 'rust.txt',
    strictness: 'balanced',
    questions: ['Why ownership?', 'What is borrowing?'],
    grades: [],
    gaps: [],
    canRemediate: false,
    isTrace: false,
    thinking: true,
    elapsed: 2,
  );

  const resultsPassed = RemoteExam(
    phase: 'results',
    deck: 'rust.txt',
    strictness: 'balanced',
    questions: ['Why ownership?', 'What is borrowing?'],
    passed: true,
    grades: [
      RemoteExamGrade(
        question: 'Why ownership?',
        points: ['memory safety without a GC'],
        answer: 'a1',
        verdict: 'PASS',
        feedback: 'Right on.',
        missed: [],
      ),
    ],
    gaps: [],
    canRemediate: false,
    isTrace: false,
    thinking: false,
  );

  const resultsFailed = RemoteExam(
    phase: 'results',
    deck: 'rust.txt',
    strictness: 'balanced',
    questions: ['Why ownership?'],
    passed: false,
    grades: [
      RemoteExamGrade(
        question: 'Why ownership?',
        points: ['memory safety without a GC'],
        answer: 'it has a GC',
        verdict: 'FAIL',
        feedback: 'Rust has no GC.',
        missed: ['memory safety without a GC'],
      ),
    ],
    gaps: ['ownership and the GC-free memory model'],
    canRemediate: true,
    isTrace: false,
    thinking: false,
  );

  const remediating = RemoteExam(
    phase: 'remediating',
    deck: 'rust.txt',
    strictness: 'balanced',
    questions: ['Why ownership?'],
    passed: false,
    grades: [],
    gaps: ['ownership and the GC-free memory model'],
    canRemediate: true,
    isTrace: false,
    thinking: true,
  );

  const remediated = RemoteExam(
    phase: 'remediated',
    deck: 'rust.txt',
    strictness: 'balanced',
    questions: ['Why ownership?'],
    passed: false,
    grades: [],
    gaps: ['ownership and the GC-free memory model'],
    canRemediate: false,
    cards: '# q?\n\ta\n',
    isTrace: false,
    thinking: false,
  );

  testWidgets('full pass walk: generating -> answering -> grading -> results, applyPassed exactly once',
      (tester) async {
    final client = FakeServerClient(
      getReplies: [generating, answering2, grading, resultsPassed],
    );
    final passedCalls = <BigInt>[];
    await pumpExam(tester, client: client, passedCalls: passedCalls);

    // examStart -> the first examGet (generating).
    await tester.pump();
    expect(find.textContaining('The server is working'), findsOneWidget);

    // Tick to answering; polling stops (thinking:false, not a working phase).
    await tester.pump(_pollInterval);
    expect(find.text('Question 1 of 2'), findsOneWidget);

    await tester.enterText(find.byKey(const ValueKey('exam-answer-field')), 'a1');
    await tester.tap(find.text('Next'));
    await tester.pump();
    expect(find.text('Question 2 of 2'), findsOneWidget);

    await tester.enterText(find.byKey(const ValueKey('exam-answer-field')), 'a2');
    await tester.tap(find.text('Submit'));
    await tester.pump();
    // Submit's own follow-up poll lands on grading, scheduling the timer.
    expect(find.textContaining('The server is working'), findsOneWidget);

    await tester.pump(_pollInterval);
    await tester.pumpAndSettle();

    expect(find.text('Passed.'), findsOneWidget);
    expect(find.text('Right on.'), findsOneWidget);
    expect(passedCalls, hasLength(1));
    expect(passedCalls.single, BigInt.from(1000));

    expect(client.gradeAnswers, hasLength(1));
    expect(client.gradeAnswers.single, ['a1', 'a2']);
  });

  testWidgets('applyPassed still applies exactly once when two in-flight polls both resolve to results',
      (tester) async {
    // Two ticks of the periodic timer both fire (and both start a poll)
    // before either poll resolves: the same shape as a real server reply
    // arriving slower than the poll interval. Both polls settle on the
    // same terminal `results` DTO, matching what a real desktop would
    // still report on a second concurrent GET once the sitting is done.
    final gate0 = Completer<RemoteExam?>();
    final gate1 = Completer<RemoteExam?>();
    final client = FakeServerClient(
      getReplies: const [grading],
      getGates: {1: gate0, 2: gate1},
    );
    final passedCalls = <BigInt>[];
    await pumpExam(tester, client: client, passedCalls: passedCalls);
    await tester.pump(); // examStart -> call 0: grading (schedules the timer)
    expect(find.textContaining('The server is working'), findsOneWidget);

    await tester.pump(_pollInterval); // tick #1 -> call 1, parked on gate0
    await tester.pump(_pollInterval); // tick #2 -> call 2, parked on gate1

    gate0.complete(resultsPassed);
    await tester.pump();
    await tester.pump();
    expect(find.text('Passed.'), findsOneWidget, reason: 'the first settled poll renders results');

    gate1.complete(resultsPassed);
    await tester.pump();
    await tester.pump();

    expect(passedCalls, hasLength(1), reason: 'the second, already-in-flight poll must not double-apply');
  });

  testWidgets('answering navigation keeps typed text across Back/Next', (tester) async {
    final client = FakeServerClient(getReplies: const [answering2]);
    await pumpExam(tester, client: client);
    await tester.pump();

    expect(find.text('Question 1 of 2'), findsOneWidget);
    await tester.enterText(find.byKey(const ValueKey('exam-answer-field')), 'a1');
    await tester.tap(find.text('Next'));
    await tester.pump();

    expect(find.text('Question 2 of 2'), findsOneWidget);
    await tester.enterText(find.byKey(const ValueKey('exam-answer-field')), 'a2');
    await tester.tap(find.text('Back'));
    await tester.pump();

    expect(find.text('Question 1 of 2'), findsOneWidget);
    final field = tester.widget<TextField>(find.byKey(const ValueKey('exam-answer-field')));
    expect(field.controller?.text, 'a1', reason: 'a per-question controller keeps its own text');

    await tester.tap(find.text('Next'));
    await tester.pump();
    await tester.tap(find.text('Submit'));
    await tester.pump();

    expect(client.gradeAnswers.single, ['a1', 'a2']);
  });

  testWidgets('fail -> remediate -> cards: applyRemediation gets the cards verbatim, SnackBar shows the count',
      (tester) async {
    final client = FakeServerClient(
      getReplies: [resultsFailed, remediating, remediated],
      remediateReplies: const [true],
    );
    final remediationCalls = <(String, BigInt)>[];
    await pumpExam(
      tester,
      client: client,
      remediationCalls: remediationCalls,
      remediationCountReply: 3,
    );
    await tester.pump();

    expect(find.text('Not yet.'), findsOneWidget);
    expect(find.text('Turn the gaps into cards'), findsOneWidget);

    await tester.tap(find.text('Turn the gaps into cards'));
    await tester.pump();
    expect(find.textContaining('The server is working'), findsOneWidget);

    await tester.pump(_pollInterval);
    expect(find.text('Done.'), findsOneWidget);

    expect(remediationCalls, hasLength(1));
    expect(remediationCalls.single.$1, '# q?\n\ta\n');
    expect(find.text('3 new cards to drill.'), findsOneWidget);
  });

  testWidgets('refused start: examStart false shows the refusal SnackBar and pops', (tester) async {
    final client = FakeServerClient(startReply: false);
    final navigatorKey = GlobalKey<NavigatorState>();
    await tester.pumpWidget(MaterialApp(
      navigatorKey: navigatorKey,
      home: Scaffold(
        body: Builder(
          builder: (context) => ElevatedButton(
            onPressed: () => Navigator.of(context).push(MaterialPageRoute(
              builder: (_) => ExamScreen(
                deckName: 'rust.txt',
                client: client,
                applyPassed: (_) {},
                applyRemediation: (_, _) => 0,
                nowMs: () => BigInt.from(1000),
                pollInterval: _pollInterval,
              ),
            )),
            child: const Text('open'),
          ),
        ),
      ),
    ));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();

    expect(find.text('The desktop refused the exam.'), findsOneWidget);
    expect(find.byType(ExamScreen), findsNothing);
  });

  testWidgets('PairingExpired on start shows the exact re-pair SnackBar and pops', (tester) async {
    final client = FakeServerClient(expireOnStart: true);
    await tester.pumpWidget(MaterialApp(
      home: Scaffold(
        body: Builder(
          builder: (context) => ElevatedButton(
            onPressed: () => Navigator.of(context).push(MaterialPageRoute(
              builder: (_) => ExamScreen(
                deckName: 'rust.txt',
                client: client,
                applyPassed: (_) {},
                applyRemediation: (_, _) => 0,
                nowMs: () => BigInt.from(1000),
                pollInterval: _pollInterval,
              ),
            )),
            child: const Text('open'),
          ),
        ),
      ),
    ));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();

    expect(
      find.text('Pairing expired. Pair again from the deck list menu.'),
      findsOneWidget,
    );
    expect(find.byType(ExamScreen), findsNothing);
  });

  testWidgets('submit failure: examGrade false shows the SnackBar and stays on the last question',
      (tester) async {
    const oneQuestion = RemoteExam(
      phase: 'answering',
      deck: 'rust.txt',
      strictness: 'balanced',
      questions: ['Why ownership?'],
      grades: [],
      gaps: [],
      canRemediate: false,
      isTrace: false,
      thinking: false,
    );
    final client = FakeServerClient(getReplies: const [oneQuestion], gradeReplies: const [false]);
    await pumpExam(tester, client: client);
    await tester.pump();

    expect(find.text('Question 1 of 1'), findsOneWidget);
    await tester.enterText(find.byKey(const ValueKey('exam-answer-field')), 'a1');
    await tester.tap(find.text('Submit'));
    await tester.pumpAndSettle();

    expect(find.text('Could not submit. Try again.'), findsOneWidget);
    expect(find.text('Question 1 of 1'), findsOneWidget);
    final field = tester.widget<TextField>(find.byKey(const ValueKey('exam-answer-field')));
    expect(field.controller?.text, 'a1');
  });
}
