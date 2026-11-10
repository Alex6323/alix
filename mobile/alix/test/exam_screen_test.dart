// Widget tests for the exam screen: a fake ServerClient (no network, no
// Rust dylib) drives examStart/examGet/examGrade/examRemediate/examClose,
// and recording fake callbacks stand in for the bridge's applyExamResult/
// applyExamRemediation. Poll interval is shrunk well below the default so
// `tester.pump` can step through the fake's canned phase-walk without a
// long real wait (Timer.periodic only advances when pumped).
import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge.dart' show AnyhowException;
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/exam_screen.dart';
import 'package:alix_mobile/server_client.dart';

import 'support/fake_server_client.dart';

const _pollInterval = Duration(milliseconds: 10);

void main() {
  /// A fresh temp dir standing in for the app support dir the "Re-pair"
  /// action would reopen the pairing sheet against; this suite never
  /// exercises a real pair, so its contents never matter beyond existing.
  Directory tempSupport() {
    final dir = Directory.systemTemp.createTempSync('alix-exam-support-');
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  Future<void> pumpExam(
    WidgetTester tester, {
    required ServerClient client,
    List<(RemoteExam, BigInt)>? resultCalls,
    List<String>? cards,
    List<String> remediationCalls = const [],
    int remediationCountReply = 1,
    String? standingReply,
    bool examChanged = false,
    List<String> unsourcedTopics = const [],
    RemoteExam? initialExam,
    Map<String, dynamic>? ideas,
    List<String> Function(List<String>)? loadAnswers,
    void Function(List<String>, List<String>)? saveAnswers,
    VoidCallback? clearAnswers,
    void Function(Map<String, dynamic>)? saveIdeas,
    ({String text, bool examChanged})? Function(RemoteExam, BigInt)? applyResult,
  }) async {
    await tester.pumpWidget(MaterialApp(
      home: ExamScreen(
        deckName: 'rust.md',
        client: client,
        support: tempSupport(),
        buildClient: (_) => client,
        initialExam: initialExam,
        ideas: ideas,
        loadAnswers: loadAnswers,
        saveAnswers: saveAnswers,
        clearAnswers: clearAnswers,
        saveIdeas: saveIdeas,
        applyResult: applyResult ?? (exam, nowMs) {
          resultCalls?.add((exam, nowMs));
          final text = standingReply;
          return text == null ? null : (text: text, examChanged: examChanged);
        },
        cards: cards,
        unsourcedTopics: unsourcedTopics,
        applyRemediation: (exam, topic) {
          remediationCalls.add('${topic.topicId}: ${topic.cards}');
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
    deck: 'rust.md',
    strictness: 'balanced',
    questions: ['Why ownership?', 'What is borrowing?'],
    grades: [],
    gaps: [],
    canRemediate: false,
    thinking: false,
  );

  const generating = RemoteExam(
    phase: 'generating',
    deck: 'rust.md',
    strictness: 'balanced',
    questions: [],
    grades: [],
    gaps: [],
    canRemediate: false,
    thinking: true,
    elapsed: 1,
  );

  const grading = RemoteExam(
    phase: 'grading',
    deck: 'rust.md',
    strictness: 'balanced',
    questions: ['Why ownership?', 'What is borrowing?'],
    grades: [],
    gaps: [],
    canRemediate: false,
    thinking: true,
    elapsed: 2,
  );

  const resultsMastered = RemoteExam(
    phase: 'results',
    deck: 'rust.md',
    strictness: 'balanced',
    questions: ['Why ownership?', 'What is borrowing?'],
    topics: [
      RemoteExamTopic(
        topicId: 'topic-0wnersh1pt0p1c0000000000',
        fingerprint: 'fp-ownership',
        label: 'Ownership',
        cards: ['card-a'],
        pass: true,
        verdicts: ExamVerdicts(pass: 1, partial: 0, total: 1),
      ),
    ],
    verdicts: ExamVerdicts(pass: 1, partial: 0, total: 1),
    full: true,
    mastered: true,
    grades: [
      RemoteExamGrade(
        question: 'Why ownership?',
        points: ['memory safety without a GC'],
        answer: 'a1',
        verdict: 'PASS',
        feedback: 'Right on.',
        missed: [],
        shape: 'instance',
      ),
    ],
    gaps: [],
    canRemediate: false,
    thinking: false,
  );

  const resultsFailed = RemoteExam(
    phase: 'results',
    deck: 'rust.md',
    strictness: 'balanced',
    questions: ['Why ownership?'],
    topics: [
      RemoteExamTopic(
        topicId: 'topic-0wnersh1pt0p1c0000000000',
        fingerprint: 'fp-ownership',
        label: 'Ownership',
        cards: ['card-a'],
        pass: false,
        verdicts: ExamVerdicts(pass: 1, partial: 1, total: 2),
      ),
    ],
    verdicts: ExamVerdicts(pass: 1, partial: 1, total: 2),
    full: false,
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
    thinking: false,
  );

  const remediating = RemoteExam(
    phase: 'remediating',
    deck: 'rust.md',
    strictness: 'balanced',
    questions: ['Why ownership?'],
    grades: [],
    gaps: ['ownership and the GC-free memory model'],
    canRemediate: true,
    thinking: true,
  );

  const remediated = RemoteExam(
    phase: 'remediated',
    deck: 'rust.md',
    strictness: 'balanced',
    questions: ['Why ownership?'],
    grades: [],
    gaps: ['ownership and the GC-free memory model'],
    canRemediate: false,
    cardsByTopic: [
      RemoteTopicCards(topicId: 'topic-a', cards: '## q?\na\n'),
      RemoteTopicCards(topicId: 'topic-b', cards: '## r?\nb\n'),
    ],
    thinking: false,
  );

  testWidgets('full pass walk: generating -> answering -> grading -> results, applyResult exactly once',
      (tester) async {
    final client = FakeServerClient(
      examGetReplies: [generating, answering2, grading, resultsMastered],
    );
    final resultCalls = <(RemoteExam, BigInt)>[];
    await pumpExam(tester,
        client: client, resultCalls: resultCalls, standingReply: 'mastered (4 Nov)');

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

    expect(find.text('1 / 1'), findsWidgets);
    expect(find.text('mastered (4 Nov)'), findsOneWidget,
        reason: 'the standing the phone recorded shows under the score');
    expect(find.text('Right on.'), findsOneWidget);
    expect(find.text('instance'), findsOneWidget);
    expect(resultCalls, hasLength(1));
    expect(resultCalls.single.$1, resultsMastered);
    expect(resultCalls.single.$2, BigInt.from(1000));

    expect(client.gradeAnswers, hasLength(1));
    expect(client.gradeAnswers.single, ['a1', 'a2']);
  });

  testWidgets('a changed mark underlines dashed and unsourced topics list on the results',
      (tester) async {
    final client = FakeServerClient(
      examGetReplies: [generating, answering2, grading, resultsMastered],
    );
    await pumpExam(tester,
        client: client,
        standingReply: 'mastered (4 Nov)',
        examChanged: true,
        unsourcedTopics: const ['Terms', 'Intro']);
    await tester.pump();
    await tester.pump(_pollInterval);
    await tester.enterText(find.byKey(const ValueKey('exam-answer-field')), 'a1');
    await tester.tap(find.text('Next'));
    await tester.pump();
    await tester.enterText(find.byKey(const ValueKey('exam-answer-field')), 'a2');
    await tester.tap(find.text('Submit'));
    await tester.pump();
    await tester.pump(_pollInterval);
    await tester.pumpAndSettle();

    final mark = tester.widget<Text>(find.text('mastered (4 Nov)'));
    expect(mark.style?.decoration, TextDecoration.underline);
    expect(mark.style?.decorationStyle, TextDecorationStyle.dashed);
    expect(find.text('Not examined: Terms, Intro'), findsOneWidget);
  });

  testWidgets('applyResult still applies exactly once when two in-flight polls both resolve to results',
      (tester) async {
    // Two ticks of the periodic timer both fire (and both start a poll)
    // before either poll resolves: the same shape as a real server reply
    // arriving slower than the poll interval. Both polls settle on the
    // same terminal `results` DTO, matching what a real desktop would
    // still report on a second concurrent GET once the sitting is done.
    final gate0 = Completer<RemoteExam?>();
    final gate1 = Completer<RemoteExam?>();
    final client = FakeServerClient(
      examGetReplies: const [grading],
      examGetGates: {1: gate0, 2: gate1},
    );
    final resultCalls = <(RemoteExam, BigInt)>[];
    await pumpExam(tester, client: client, resultCalls: resultCalls);
    await tester.pump(); // examStart -> call 0: grading (schedules the timer)
    expect(find.textContaining('The server is working'), findsOneWidget);

    await tester.pump(_pollInterval); // tick #1 -> call 1, parked on gate0
    await tester.pump(_pollInterval); // tick #2 -> call 2, parked on gate1

    gate0.complete(resultsMastered);
    await tester.pump();
    await tester.pump();
    expect(find.text('Right on.'), findsOneWidget, reason: 'the first settled poll renders results');

    gate1.complete(resultsMastered);
    await tester.pump();
    await tester.pump();

    expect(resultCalls, hasLength(1), reason: 'the second, already-in-flight poll must not double-apply');
  });

  testWidgets(
      'a failed exam leaves the remediate path available', (tester) async {
    final client = FakeServerClient(examGetReplies: const [resultsFailed]);
    await pumpExam(tester, client: client);
    await tester.pump();

    expect(find.text('1.5 / 2'), findsWidgets);
    expect(find.textContaining('mastered ('), findsNothing);
    expect(find.textContaining('passed ('), findsNothing);
    expect(find.text('Turn the gaps into cards'), findsOneWidget);
  });

  testWidgets('a failed preparation shows the server error text', (tester) async {
    const failed = RemoteExam(
      phase: 'generating',
      deck: 'rust.md',
      strictness: 'balanced',
      questions: [],
      grades: [],
      gaps: [],
      canRemediate: false,
      thinking: false,
      error: 'cannot read /src: Is a directory (os error 21)',
    );
    final client = FakeServerClient(examGetReplies: const [failed]);
    await pumpExam(tester, client: client);
    await tester.pump();

    expect(find.text('The exam backend failed.'), findsOneWidget);
    expect(
      find.text('cannot read /src: Is a directory (os error 21)'),
      findsOneWidget,
    );
  });

  testWidgets('results show a per-topic count and mark', (tester) async {
    final client = FakeServerClient(examGetReplies: const [resultsFailed]);
    await pumpExam(tester, client: client);
    await tester.pump();

    expect(find.text('Ownership'), findsOneWidget);
    expect(find.text('needs work'), findsOneWidget);
    expect(find.text('pass'), findsNothing);
  });

  testWidgets('no result wording says passed or failed', (tester) async {
    for (final reply in const [resultsMastered, resultsFailed]) {
      final client = FakeServerClient(examGetReplies: [reply]);
      await pumpExam(tester, client: client);
      await tester.pump();

      expect(find.textContaining(RegExp('passed|failed', caseSensitive: false)), findsNothing);
      await tester.pumpWidget(const SizedBox.shrink());
    }
  });

  testWidgets('a failed sitting records its result too, once', (tester) async {
    final client = FakeServerClient(examGetReplies: const [resultsFailed]);
    final resultCalls = <(RemoteExam, BigInt)>[];
    await pumpExam(tester, client: client, resultCalls: resultCalls);
    await tester.pump();

    expect(resultCalls, hasLength(1));
    expect(resultCalls.single.$1.full, isFalse);
  });

  testWidgets('a filtered sitting starts with its cards and a full one with none', (tester) async {
    final filtered = FakeServerClient(examGetReplies: const [answering2]);
    await pumpExam(tester, client: filtered, cards: const ['card-a', 'card-b']);
    await tester.pump();
    expect(filtered.startedCards, ['card-a', 'card-b']);

    await tester.pumpWidget(const SizedBox.shrink());
    final full = FakeServerClient(examGetReplies: const [answering2]);
    await pumpExam(tester, client: full);
    await tester.pump();
    expect(full.startedCards, isNull);
  });

  testWidgets('first-poll-null: a slow start recovers once the sitting is ready', (tester) async {
    final client = FakeServerClient(
      nullFirstExamGetCalls: 1,
      examGetReplies: const [answering2],
    );
    await pumpExam(tester, client: client);
    await tester.pump(); // examStart -> first examGet: null, schedules a bounded retry
    expect(find.text('Question 1 of 2'), findsNothing, reason: 'still stranded until the retry lands');
    expect(find.text('The desktop refused the exam.'), findsNothing,
        reason: 'a single null must retry, not give up immediately');

    await tester.pump(_pollInterval); // the retry fires -> examGet: answering2

    expect(find.text('Question 1 of 2'), findsOneWidget);
  });

  testWidgets(
      'first-poll-null: a persistently-null server retries before falling back to the refusal path '
      '(no infinite spinner)', (tester) async {
    final client = FakeServerClient(); // examGetReplies empty: examGet always resolves to null
    final navigatorKey = GlobalKey<NavigatorState>();
    await tester.pumpWidget(MaterialApp(
      navigatorKey: navigatorKey,
      home: Scaffold(
        body: Builder(
          builder: (context) => ElevatedButton(
            onPressed: () => Navigator.of(context).push(MaterialPageRoute(
              builder: (_) => ExamScreen(
                deckName: 'rust.md',
                client: client,
                support: tempSupport(),
                buildClient: (_) => client,
                applyResult: (_, _) => null,
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
    await tester.pump(); // push -> examStart -> first examGet: null, schedules a retry

    // One retry tick: still bounded, not yet given up.
    await tester.pump(_pollInterval);
    expect(find.text('The desktop refused the exam.'), findsNothing,
        reason: 'a single null must retry, not give up immediately');

    // Exhaust the bound (comfortably more ticks than the retry limit).
    for (var i = 0; i < 6; i++) {
      await tester.pump(_pollInterval);
    }
    await tester.pumpAndSettle();

    expect(find.text('The desktop refused the exam.'), findsOneWidget);
    expect(find.byType(ExamScreen), findsNothing);
  });

  testWidgets('answering navigation keeps typed text across Back/Next', (tester) async {
    final client = FakeServerClient(examGetReplies: const [answering2]);
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

  testWidgets('moving on persists and locks an answer, and reopening starts at the first empty one', (
    tester,
  ) async {
    final saved = <List<String>>[];
    final firstClient = FakeServerClient(examGetReplies: const [answering2]);
    await pumpExam(
      tester,
      client: firstClient,
      saveAnswers: (_, answers) => saved.add(List.of(answers)),
    );
    await tester.pump();

    await tester.enterText(find.byKey(const ValueKey('exam-answer-field')), 'first answer');
    await tester.tap(find.text('Next'));
    await tester.pump();
    expect(saved, [
      ['first answer', ''],
    ]);
    await tester.tap(find.text('Back'));
    await tester.pump();
    final locked = tester.widget<TextField>(find.byKey(const ValueKey('exam-answer-field')));
    expect(locked.readOnly, isTrue);

    await tester.pumpWidget(const SizedBox.shrink());
    final resumedClient = FakeServerClient();
    await pumpExam(
      tester,
      client: resumedClient,
      initialExam: answering2,
      loadAnswers: (_) => const ['first answer', ''],
    );
    await tester.pump();

    expect(resumedClient.startedDeck, isNull, reason: 'a ready slot is resumed, not restarted');
    expect(find.text('Question 2 of 2'), findsOneWidget);
    await tester.tap(find.text('Back'));
    await tester.pump();
    final resumedLocked = tester.widget<TextField>(find.byKey(const ValueKey('exam-answer-field')));
    expect(resumedLocked.controller?.text, 'first answer');
    expect(resumedLocked.readOnly, isTrue);
  });

  testWidgets('leaving generation or answering preserves the remote slot', (tester) async {
    for (final phase in const [generating, answering2]) {
      final client = FakeServerClient(examGetReplies: [phase]);
      await pumpExam(tester, client: client);
      await tester.pump();
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pump();
      expect(client.examCloseCalls, 0, reason: '${phase.phase} survives screen dispose');
    }
  });

  testWidgets('leaving the results closes the remote slot', (tester) async {
    final client = FakeServerClient(examGetReplies: [resultsFailed]);
    await pumpExam(tester, client: client);
    await tester.pump();
    expect(find.text('needs work'), findsOneWidget, reason: 'step 1: results shown');
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pump();
    expect(client.examCloseCalls, 1, reason: 'step 2: a graded sitting is closed on leaving');
    expect(client.closed, isTrue, reason: 'step 3: the screen closes its client after the slot');
  });

  testWidgets('closing the results releases the slot before the opener resumes', (tester) async {
    for (final leave in ['close button', 'system back']) {
      final client = FakeServerClient(examGetReplies: [resultsFailed]);
      int? closesWhenResumed;
      await tester.pumpWidget(MaterialApp(
        home: Builder(
          builder: (context) => TextButton(
            onPressed: () async {
              await Navigator.of(context).push(MaterialPageRoute(
                builder: (_) => ExamScreen(
                  deckName: 'rust.md',
                  client: client,
                  support: tempSupport(),
                  buildClient: (_) => client,
                  applyResult: (exam, nowMs) => null,
                  applyRemediation: (_, _) => 0,
                  nowMs: () => BigInt.from(1000),
                  pollInterval: _pollInterval,
                ),
              ));
              closesWhenResumed = client.examCloseCalls;
            },
            child: const Text('open'),
          ),
        ),
      ));
      await tester.tap(find.text('open'));
      await tester.pumpAndSettle();
      expect(find.text('needs work'), findsOneWidget, reason: '$leave step 1: results shown');

      if (leave == 'close button') {
        await tester.tap(find.byIcon(Icons.close));
      } else {
        await tester.binding.handlePopRoute();
      }
      await tester.pumpAndSettle();

      expect(closesWhenResumed, 1, reason: '$leave step 2: the slot is closed before the opener polls');
      expect(client.examCloseCalls, 1, reason: '$leave step 3: closed exactly once');
      await tester.pumpWidget(const SizedBox.shrink());
    }
  });

  testWidgets('a second back cannot resume the opener while an explicit close is in flight', (
    tester,
  ) async {
    for (final (exam, closeControl) in [
      (resultsFailed, find.byIcon(Icons.close)),
      (answering2, find.text('End exam')),
    ]) {
    final closeGate = Completer<void>();
    final client = FakeServerClient(
      examGetReplies: [exam],
      examCloseGate: closeGate,
    );
    var resumed = false;
    await tester.pumpWidget(MaterialApp(
      home: Builder(
        builder: (context) => TextButton(
          onPressed: () async {
            await Navigator.of(context).push(MaterialPageRoute(
              builder: (_) => ExamScreen(
                deckName: 'rust.md',
                client: client,
                support: tempSupport(),
                buildClient: (_) => client,
                applyResult: (exam, nowMs) => null,
                applyRemediation: (_, _) => 0,
                nowMs: () => BigInt.from(1000),
                pollInterval: _pollInterval,
              ),
            ));
            resumed = true;
          },
          child: const Text('open'),
        ),
      ),
    ));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    await tester.tap(closeControl);
    await tester.pump();
    expect(client.examCloseCalls, 1, reason: '${exam.phase} step 1: the awaited close started');

    await tester.binding.handlePopRoute();
    await tester.pump();

    expect(resumed, isFalse, reason: '${exam.phase} step 2: a second back must stay gated until close finishes');
    expect(find.byType(ExamScreen), findsOneWidget);

    closeGate.complete();
    await tester.pumpAndSettle();
    expect(resumed, isTrue, reason: '${exam.phase} step 3: the opener resumes only after close finishes');
    await tester.pumpWidget(const SizedBox.shrink());
    }
  });

  testWidgets('explicit cancel closes the slot and clears local answers', (tester) async {
    final client = FakeServerClient(examGetReplies: const [generating]);
    var clears = 0;
    await pumpExam(tester, client: client, clearAnswers: () => clears++);
    await tester.pump();

    await tester.tap(find.text('Cancel preparation'));
    await tester.pumpAndSettle();

    expect(client.examCloseCalls, 1);
    expect(clears, 1);
  });

  testWidgets('explicitly ending an answering sitting closes and clears it', (tester) async {
    final client = FakeServerClient(examGetReplies: const [answering2]);
    var clears = 0;
    await pumpExam(tester, client: client, clearAnswers: () => clears++);
    await tester.pump();

    await tester.tap(find.text('End exam'));
    await tester.pumpAndSettle();

    expect(client.examCloseCalls, 1);
    expect(clears, 1);
  });

  testWidgets('results save ideas, apply the result, then clear answers', (tester) async {
    final events = <String>[];
    final client = FakeServerClient(
      examGetReplies: const [
        RemoteExam(
          phase: 'results',
          deck: 'rust.md',
          strictness: 'balanced',
          questions: ['Why ownership?'],
          grades: [],
          gaps: [],
          canRemediate: false,
          thinking: false,
          ideas: {
            'version': 1,
            'topics': <String, Object>{},
          },
        ),
      ],
    );
    await pumpExam(
      tester,
      client: client,
      resultCalls: <(RemoteExam, BigInt)>[],
      saveIdeas: (_) => events.add('ideas'),
      applyResult: (_, _) {
        events.add('result');
        return null;
      },
      clearAnswers: () => events.add('clear'),
    );
    await tester.pump();

    expect(events, ['ideas', 'result', 'clear']);
  });

  testWidgets('a result the core refuses to record shows its reason and still renders the results',
      (tester) async {
    const reason = 'the exam material changed while this exam was open';
    final client = FakeServerClient(examGetReplies: const [resultsMastered]);
    await pumpExam(
      tester,
      client: client,
      applyResult: (_, _) => throw AnyhowException(reason),
    );
    await tester.pump();

    expect(find.text(reason), findsOneWidget);
    expect(find.text('1 / 1'), findsWidgets);
  });

  testWidgets('a failed result the core refuses to record offers no remediation',
      (tester) async {
    const reason = 'the exam material changed while this exam was open';
    final client = FakeServerClient(examGetReplies: const [resultsFailed]);
    await pumpExam(
      tester,
      client: client,
      applyResult: (_, _) => throw AnyhowException(reason),
    );
    await tester.pump();

    expect(find.text(reason), findsOneWidget, reason: 'the refusal is shown');
    expect(find.text('1.5 / 2'), findsWidgets, reason: 'the results still render');
    expect(find.text('Turn the gaps into cards'), findsNothing,
        reason: 'gaps from voided material must not become cards');
  });

  testWidgets('a new sitting sends the saved ideas document', (tester) async {
    const ideas = {
      'version': 1,
      'topics': <String, Object>{},
    };
    final client = FakeServerClient(examGetReplies: const [generating]);
    await pumpExam(tester, client: client, ideas: ideas);
    await tester.pump();

    expect(client.startedIdeas, ideas);
  });

  testWidgets('fail -> remediate -> cards: applyRemediation gets each topic\'s cards verbatim, SnackBar shows the sum',
      (tester) async {
    final client = FakeServerClient(
      examGetReplies: [resultsFailed, remediating, remediated],
      examRemediateReplies: const [true],
    );
    final remediationCalls = <String>[];
    await pumpExam(
      tester,
      client: client,
      remediationCalls: remediationCalls,
      remediationCountReply: 3,
    );
    await tester.pump();

    expect(find.text('Turn the gaps into cards'), findsOneWidget);

    await tester.tap(find.text('Turn the gaps into cards'));
    await tester.pump();
    expect(find.textContaining('The server is working'), findsOneWidget);

    await tester.pump(_pollInterval);
    expect(find.text('Done.'), findsOneWidget);

    expect(remediationCalls, ['topic-a: ## q?\na\n', 'topic-b: ## r?\nb\n']);
    expect(find.text('6 new cards to drill.'), findsOneWidget);
  });

  testWidgets('refused start: examStart false shows the refusal SnackBar and pops', (tester) async {
    final client = FakeServerClient(examStartReply: false);
    final navigatorKey = GlobalKey<NavigatorState>();
    await tester.pumpWidget(MaterialApp(
      navigatorKey: navigatorKey,
      home: Scaffold(
        body: Builder(
          builder: (context) => ElevatedButton(
            onPressed: () => Navigator.of(context).push(MaterialPageRoute(
              builder: (_) => ExamScreen(
                deckName: 'rust.md',
                client: client,
                support: tempSupport(),
                buildClient: (_) => client,
                applyResult: (_, _) => null,
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

  testWidgets(
      'PairingExpired on start shows the exact re-pair SnackBar and pops; '
      'tapping Re-pair opens the pairing sheet on a live context', (tester) async {
    final client = FakeServerClient(expireOnExamStart: true);
    await tester.pumpWidget(MaterialApp(
      home: Scaffold(
        body: Builder(
          builder: (context) => ElevatedButton(
            onPressed: () => Navigator.of(context).push(MaterialPageRoute(
              builder: (_) => ExamScreen(
                deckName: 'rust.md',
                client: client,
                support: tempSupport(),
                buildClient: (_) => client,
                applyResult: (_, _) => null,
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
      find.text('Pairing expired. Pair again from Settings → Connected devices.'),
      findsOneWidget,
    );
    expect(find.byType(ExamScreen), findsNothing);
    expect(find.text('Re-pair'), findsOneWidget);

    // The exam screen has already popped (its own context is dead); the
    // action must still open the pairing sheet safely on the surviving
    // root context, not throw.
    await tester.tap(find.text('Re-pair'));
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('pairing-url-field')), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('submit failure: examGrade false shows the SnackBar and stays on the last question',
      (tester) async {
    const oneQuestion = RemoteExam(
      phase: 'answering',
      deck: 'rust.md',
      strictness: 'balanced',
      questions: ['Why ownership?'],
      grades: [],
      gaps: [],
      canRemediate: false,
      thinking: false,
    );
    final client = FakeServerClient(examGetReplies: const [oneQuestion], examGradeReplies: const [false]);
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

  test('examDeckName resolves a workspace member to <workspace>/<file>', () {
    expect(examDeckName(deckPath: '/r/wsfolder/decks/member.md', rootDir: '/r'), 'wsfolder/member.md');
    expect(examDeckName(deckPath: '/r/rust.md', rootDir: '/r'), 'rust.md');
  });
}
