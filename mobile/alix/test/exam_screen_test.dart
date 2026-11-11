// Widget tests for the exam screen: a fake ServerClient (no network, no
// Rust dylib) drives examStart/examGet/examGrade/examRemediate/examClose,
// and recording fake callbacks stand in for the bridge's applyExamResult/
// applyExamRemediation. Poll interval is shrunk well below the default so
// `tester.pump` can step through the fake's canned phase-walk without a
// long real wait (Timer.periodic only advances when pumped).
import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
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
    int? Function(RemoteExam, RemoteTopicCards)? applyRemediation,
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
        applyRemediation: applyRemediation ??
            (exam, topic) {
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

  testWidgets('a long question with a code block leaves typing room above the phone keyboard', (tester) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3;
    tester.view.viewInsets = const FakeViewPadding(bottom: 900);
    addTearDown(tester.view.reset);
    const prose = 'For one commit, A and B each build a checkpoint whose summary digest '
        'is X. C and D each build a checkpoint whose summary digest is Y, because their '
        'post-consensus verdict on one transaction differed. Every validator signs its own '
        'build and submits that signature to consensus. What does the checkpoint aggregator '
        'conclude for this commit, and what happens next on A, B, C and D? Say which '
        'quantity decides the outcome. Then repeat the analysis for a committee where C '
        'holds 4,000 and A holds 2,000, name the threshold the aggregator compares against, '
        'state whether any checkpoint for this commit is ever certified, and explain what '
        'an operator would see in the logs of each validator while the split persists.';
    final exam = RemoteExam.fromJson({
      'phase': 'answering',
      'deck': 'rust.md',
      'questions': [
        'A committee of four validators holds this stake out of a total voting power of 10,000:\n\n'
            '```\nvalidator A: 4,000\nvalidator B: 3,000\nvalidator C: 2,000\nvalidator D: 1,000\n```\n\n$prose',
      ],
      'questions_units': [
        [
          {
            'kind': 'sentence',
            'text': 'A committee of four validators holds this stake out of a total voting power of 10,000:',
            'runs': [
              {'text': 'A committee of four validators holds this stake out of a total voting power of 10,000:'},
            ],
          },
          {
            'kind': 'code',
            'lines': ['validator A: 4,000', 'validator B: 3,000', 'validator C: 2,000', 'validator D: 1,000'],
          },
          {
            'kind': 'sentence',
            'text': prose,
            'runs': [
              {'text': prose},
            ],
          },
        ],
      ],
      'thinking': false,
    });
    final client = FakeServerClient(examGetReplies: [exam]);
    await pumpExam(tester, client: client);
    await tester.pump();

    expect(find.textContaining('```'), findsNothing, reason: 'the question renders its units, not raw markdown');
    final screen = tester.view.physicalSize / tester.view.devicePixelRatio;
    final keyboardTop = screen.height - tester.view.viewInsets.bottom / tester.view.devicePixelRatio;
    final field = tester.getRect(find.byKey(const ValueKey('exam-answer-field')));
    expect(field.height, greaterThanOrEqualTo(96), reason: 'answer field rect $field must keep typing room');
    expect(field.bottom, lessThanOrEqualTo(keyboardTop),
        reason: 'answer field rect $field must stay above keyboard top $keyboardTop');
    expect(tester.takeException(), isNull, reason: 'no layout overflow');
  });

  RemoteExam longQuestions(int count) {
    final prose = List.filled(40, 'This question is long enough to scroll on a phone.').join(' ');
    return RemoteExam.fromJson({
      'phase': 'answering',
      'deck': 'rust.md',
      'questions': [for (var i = 1; i <= count; i++) 'Start of question $i. $prose'],
      'questions_units': [
        for (var i = 1; i <= count; i++)
          [
            {
              'kind': 'sentence',
              'text': 'Start of question $i. $prose',
              'runs': [
                {'text': 'Start of question $i. $prose'},
              ],
            },
          ],
      ],
      'thinking': false,
    });
  }

  testWidgets('a long question shows a scrollbar and a hairline above the answer', (tester) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);
    await pumpExam(tester, client: FakeServerClient(examGetReplies: [longQuestions(1)]));
    await tester.pump();

    final scrollbar = tester.widget<Scrollbar>(
      find.ancestor(of: find.byKey(const ValueKey('exam-question')), matching: find.byType(Scrollbar)),
    );
    expect(scrollbar.thumbVisibility, isTrue, reason: 'the scrollbar shows without a touch');
    final question = tester.getRect(find.byKey(const ValueKey('exam-question')));
    final hairline = tester.getRect(find.byKey(const ValueKey('exam-question-hairline')));
    final field = tester.getRect(find.byKey(const ValueKey('exam-answer-field')));
    expect(hairline.top, greaterThanOrEqualTo(question.bottom), reason: 'hairline $hairline below question $question');
    expect(hairline.bottom, lessThanOrEqualTo(field.top), reason: 'hairline $hairline above field $field');
  });

  testWidgets('the next question starts at its top, not at the last scroll offset', (tester) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);
    await pumpExam(tester, client: FakeServerClient(examGetReplies: [longQuestions(2)]));
    await tester.pump();

    await tester.drag(find.byKey(const ValueKey('exam-question')), const Offset(0, -300));
    await tester.pumpAndSettle();
    expect(find.textContaining('Start of question 1'), findsOneWidget);
    await tester.enterText(find.byKey(const ValueKey('exam-answer-field')), 'an answer');
    await tester.tap(find.text('Next'));
    await tester.pumpAndSettle();

    final question = tester.getRect(find.byKey(const ValueKey('exam-question')));
    final start = tester.getRect(find.textContaining('Start of question 2'));
    expect(start.top, greaterThanOrEqualTo(question.top),
        reason: 'question 2 text $start must begin inside the question area $question');
  });

  testWidgets('an exam code block numbers its lines so a question can cite them', (tester) async {
    final exam = RemoteExam.fromJson({
      'phase': 'answering',
      'deck': 'rust.md',
      'questions': ['What does line 3 do?\n\n```\nfn main() {\n    let x;\n    x = 1;\n}\n```'],
      'questions_units': [
        [
          {
            'kind': 'sentence',
            'text': 'What does line 3 do?',
            'runs': [
              {'text': 'What does line 3 do?'},
            ],
          },
          {
            'kind': 'code',
            'lines': ['fn main() {', '    let x;', '    x = 1;', '}'],
          },
        ],
      ],
      'thinking': false,
    });
    final client = FakeServerClient(examGetReplies: [exam]);
    await pumpExam(tester, client: client);
    await tester.pump();

    final lines = ['fn main() {', '    let x;', '    x = 1;', '}'];
    for (final (index, line) in lines.indexed) {
      final number = find.text('${index + 1}');
      expect(number, findsOneWidget, reason: 'line ${index + 1} is numbered');
      expect(
        tester.getTopLeft(number).dy,
        tester.getTopLeft(find.text(line)).dy,
        reason: 'number ${index + 1} sits on the row of "$line"',
      );
      expect(
        tester.getTopLeft(number).dx,
        lessThan(tester.getTopLeft(find.text(line)).dx),
        reason: 'number ${index + 1} sits left of its line',
      );
    }
  });

  testWidgets('large accessibility text keeps three-digit code line numbers readable', (tester) async {
    addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
    tester.platformDispatcher.textScaleFactorTestValue = 2;
    final lines = [for (var i = 1; i <= 100; i++) 'line $i'];
    final exam = RemoteExam.fromJson({
      'phase': 'answering',
      'deck': 'rust.md',
      'questions': ['What does line 100 do?'],
      'questions_units': [
        [
          {
            'kind': 'code',
            'lines': lines,
          },
        ],
      ],
      'thinking': false,
    });
    await pumpExam(tester, client: FakeServerClient(examGetReplies: [exam]));
    await tester.pump();

    final paragraph = tester.renderObject<RenderParagraph>(find.text('100'));
    final naturalSize = TextPainter(
      text: paragraph.text,
      textDirection: paragraph.textDirection,
      textScaler: paragraph.textScaler,
      locale: paragraph.locale,
    )..layout();
    expect(
      paragraph.size.width,
      greaterThanOrEqualTo(naturalSize.width),
      reason: 'line 100 must fit on one line at the user text scale',
    );
  });

  testWidgets('wrapped code keeps each number aligned with its logical source line', (tester) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);
    final wrapped = List.filled(20, 'wrapped source words').join(' ');
    final exam = RemoteExam.fromJson({
      'phase': 'answering',
      'deck': 'rust.md',
      'questions': ['What do these lines do?'],
      'questions_units': [
        [
          {
            'kind': 'code',
            'lines': [wrapped, 'short second line'],
          },
        ],
      ],
      'thinking': false,
    });
    await pumpExam(tester, client: FakeServerClient(examGetReplies: [exam]));
    await tester.pump();

    final firstNumber = tester.getRect(find.text('1'));
    final firstLine = tester.getRect(find.text(wrapped));
    final secondNumber = tester.getRect(find.text('2'));
    final secondLine = tester.getRect(find.text('short second line'));
    expect(firstNumber.top, firstLine.top, reason: 'line 1 starts beside its number');
    expect(firstLine.height, greaterThan(secondLine.height), reason: 'line 1 actually wraps');
    expect(secondNumber.top, greaterThanOrEqualTo(firstLine.bottom), reason: 'line 2 follows all of line 1');
    expect(secondNumber.top, secondLine.top, reason: 'line 2 starts beside its number');
    expect(tester.takeException(), isNull);
  });

  testWidgets('results identify each grade with its rendered question', (tester) async {
    final exam = RemoteExam.fromJson({
      'phase': 'results',
      'deck': 'rust.md',
      'questions': [
        'Which validator has the largest stake?\n\n```\nvalidator A: 4,000\nvalidator B: 3,000\n```',
      ],
      'questions_units': [
        [
          {
            'kind': 'sentence',
            'text': 'Which validator has the largest stake?',
            'runs': [
              {'text': 'Which validator has the largest stake?'},
            ],
          },
          {
            'kind': 'code',
            'lines': ['validator A: 4,000', 'validator B: 3,000'],
          },
        ],
      ],
      'verdicts': {'pass': 0, 'partial': 1, 'total': 1},
      'grades': [
        {
          'question':
              'Which validator has the largest stake?\n\n```\nvalidator A: 4,000\nvalidator B: 3,000\n```',
          'question_units': [
            {
              'kind': 'sentence',
              'text': 'Which validator has the largest stake?',
              'runs': [
                {'text': 'Which validator has the largest stake?'},
              ],
            },
            {
              'kind': 'code',
              'lines': ['validator A: 4,000', 'validator B: 3,000'],
            },
          ],
          'points': ['validator A'],
          'answer': 'A',
          'verdict': 'PARTIAL',
          'feedback': 'Name the stake as well.',
          'missed': ['4,000'],
        },
      ],
      'gaps': [],
      'can_remediate': false,
      'thinking': false,
    });
    final client = FakeServerClient(examGetReplies: [exam]);
    await pumpExam(tester, client: client);
    await tester.pump();

    expect(find.text('Which validator has the largest stake?'), findsOneWidget,
        reason: 'each verdict must identify the question it grades');
    expect(find.text('validator A: 4,000'), findsOneWidget,
        reason: 'the grade question must use the server-provided content units');
    expect(find.text('validator B: 3,000'), findsOneWidget,
        reason: 'the grade question must use the server-provided content units');
    expect(find.textContaining('```'), findsNothing,
        reason: 'results must not expose raw Markdown fences');
  });

  testWidgets('results render structured feedback and failed rubric details', (tester) async {
    final exam = RemoteExam.fromJson({
      'phase': 'results',
      'deck': 'rust.md',
      'questions': ['Which validator has the largest stake?'],
      'questions_units': [
        [
          {
            'kind': 'sentence',
            'text': 'Which validator has the largest stake?',
            'runs': [
              {'text': 'Which validator has the largest stake?'},
            ],
          },
        ],
      ],
      'verdicts': {'pass': 0, 'partial': 1, 'total': 1},
      'grades': [
        {
          'question': 'Which validator has the largest stake?',
          'question_units': [
            {
              'kind': 'sentence',
              'text': 'Which validator has the largest stake?',
              'runs': [
                {'text': 'Which validator has the largest stake?'},
              ],
            },
          ],
          'points': ['Name the validator and stake.'],
          'points_units': [
            [
              {
                'kind': 'code',
                'lines': ['complete: validator A has 4,000'],
              },
            ],
          ],
          'answer': 'A',
          'verdict': 'PARTIAL',
          'feedback': 'Almost.\n\n```\nfeedback: include 4,000\n```',
          'feedback_units': [
            {
              'kind': 'sentence',
              'text': 'Almost.',
              'runs': [
                {'text': 'Almost.'},
              ],
            },
            {
              'kind': 'code',
              'lines': ['feedback: include 4,000'],
            },
          ],
          'missed': ['4,000'],
          'missed_units': [
            [
              {
                'kind': 'code',
                'lines': ['missed: 4,000'],
              },
            ],
          ],
        },
      ],
      'gaps': [],
      'can_remediate': true,
      'thinking': false,
    });
    final client = FakeServerClient(examGetReplies: [exam]);
    await pumpExam(tester, client: client);
    await tester.pump();

    expect(find.text('feedback: include 4,000'), findsOneWidget,
        reason: 'feedback must use feedback_units');
    expect(find.text('A complete answer covers:'), findsOneWidget,
        reason: 'a partial verdict must show its rubric points');
    expect(find.text('complete: validator A has 4,000'), findsOneWidget,
        reason: 'rubric points must use points_units');
    expect(find.text('You missed:'), findsOneWidget,
        reason: 'a partial verdict must name its missed points');
    expect(find.text('missed: 4,000'), findsOneWidget,
        reason: 'missed points must use missed_units');
    expect(find.textContaining('```'), findsNothing,
        reason: 'results must not expose raw Markdown fences');
  });

  testWidgets('a result identifies its question before announcing the verdict', (tester) async {
    const question = 'Which validator has the largest stake?';
    const exam = RemoteExam(
      phase: 'results',
      deck: 'rust.md',
      strictness: 'balanced',
      questions: [question],
      verdicts: ExamVerdicts(pass: 0, partial: 1, total: 1),
      grades: [
        RemoteExamGrade(
          question: question,
          points: ['Name the validator and stake.'],
          answer: 'A',
          verdict: 'PARTIAL',
          feedback: 'Almost.',
          missed: ['4,000'],
          shape: 'boundary',
        ),
      ],
      gaps: [],
      canRemediate: true,
      thinking: false,
    );
    final client = FakeServerClient(examGetReplies: const [exam]);
    await pumpExam(tester, client: client);
    await tester.pump();

    final questionTop = tester.getTopLeft(find.text(question)).dy;
    final verdictTop = tester.getTopLeft(find.text('PARTIAL')).dy;
    expect(questionTop, lessThan(verdictTop),
        reason: 'the row must identify the question before announcing its verdict');
  });

  testWidgets('short grade unit lists fall back at the matching raw index', (tester) async {
    final exam = RemoteExam.fromJson({
      'phase': 'results',
      'deck': 'rust.md',
      'questions': ['Why?'],
      'verdicts': {'pass': 0, 'partial': 1, 'total': 1},
      'grades': [
        {
          'question': 'Why?',
          'points': ['first raw point', 'second raw point'],
          'points_units': [
            [
              {
                'kind': 'code',
                'lines': ['first structured point'],
              },
            ],
          ],
          'answer': 'Because.',
          'verdict': 'PARTIAL',
          'feedback': 'Almost.',
          'missed': ['raw missed point'],
          'missed_units': [],
        },
      ],
      'gaps': [],
      'can_remediate': true,
      'thinking': false,
    });
    final client = FakeServerClient(examGetReplies: [exam]);
    await pumpExam(tester, client: client);
    await tester.pump();

    expect(find.text('first structured point'), findsOneWidget);
    expect(find.text('first raw point'), findsNothing);
    expect(find.text('second raw point'), findsOneWidget);
    expect(find.text('raw missed point'), findsOneWidget);
  });

  testWidgets('a pass formats feedback without exposing rubric details', (tester) async {
    final exam = RemoteExam.fromJson({
      'phase': 'results',
      'deck': 'rust.md',
      'questions': ['Why?'],
      'verdicts': {'pass': 1, 'partial': 0, 'total': 1},
      'grades': [
        {
          'question': 'Why?',
          'points': ['A complete point.'],
          'points_units': [],
          'answer': 'Because.',
          'verdict': 'PASS',
          'feedback': '```\nraw feedback\n```',
          'feedback_units': [
            {
              'kind': 'code',
              'lines': ['structured feedback'],
            },
          ],
          'missed': [],
          'missed_units': [],
        },
      ],
      'gaps': [],
      'can_remediate': false,
      'thinking': false,
    });
    final client = FakeServerClient(examGetReplies: [exam]);
    await pumpExam(tester, client: client);
    await tester.pump();

    expect(find.text('structured feedback'), findsOneWidget);
    expect(find.textContaining('```'), findsNothing);
    expect(find.text('A complete answer covers:'), findsNothing);
    expect(find.text('You missed:'), findsNothing);
  });

  testWidgets('many long result questions remain reachable on a phone screen', (tester) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);
    final questions = [
      for (var i = 1; i <= 8; i++)
        'Question $i: explain how the validator set reaches a decision when several participants '
            'report different checkpoint digests, including the threshold and the next operator action.',
    ];
    final exam = RemoteExam.fromJson({
      'phase': 'results',
      'deck': 'rust.md',
      'questions': questions,
      'questions_units': [
        for (final question in questions)
          [
            {
              'kind': 'sentence',
              'text': question,
              'runs': [
                {'text': question},
              ],
            },
          ],
      ],
      'verdicts': {'pass': 8, 'partial': 0, 'total': 8},
      'grades': [
        for (final question in questions)
          {
            'question': question,
            'question_units': [
              {
                'kind': 'sentence',
                'text': question,
                'runs': [
                  {'text': question},
                ],
              },
            ],
            'points': [],
            'answer': 'Answer',
            'verdict': 'PASS',
            'feedback': 'Correct.',
            'missed': [],
          },
      ],
      'gaps': [],
      'can_remediate': false,
      'thinking': false,
    });
    final client = FakeServerClient(examGetReplies: [exam]);
    await pumpExam(tester, client: client);
    await tester.pump();

    await tester.scrollUntilVisible(
      find.text(questions.last),
      300,
      scrollable: find.byType(Scrollable).first,
    );
    expect(find.text(questions.last), findsOneWidget);
    expect(tester.takeException(), isNull, reason: 'long result rows must scroll without overflow');
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

  testWidgets('Continue later leaves an answering sitting open with its answers kept', (tester) async {
    final client = FakeServerClient(examGetReplies: const [answering2]);
    var clears = 0;
    await tester.pumpWidget(MaterialApp(
      home: Builder(
        builder: (context) => TextButton(
          onPressed: () => Navigator.of(context).push(MaterialPageRoute(
            builder: (_) => ExamScreen(
              deckName: 'rust.md',
              client: client,
              support: tempSupport(),
              buildClient: (_) => client,
              clearAnswers: () => clears++,
              applyResult: (exam, nowMs) => null,
              applyRemediation: (_, _) => 0,
              nowMs: () => BigInt.from(1000),
              pollInterval: _pollInterval,
            ),
          )),
          child: const Text('open'),
        ),
      ),
    ));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    expect(find.text('End exam'), findsNothing, reason: 'step 1: no control discards the sitting');

    await tester.tap(find.text('Continue later'));
    await tester.pumpAndSettle();

    expect(find.byType(ExamScreen), findsNothing, reason: 'step 2: the screen is left');
    expect(client.examCloseCalls, 0, reason: 'step 3: the remote slot stays open');
    expect(clears, 0, reason: 'step 4: saved answers stay');
  });

  testWidgets('every way out of an answering exam saves the answer currently being edited', (tester) async {
    for (final leave in ['Continue later', 'close button', 'system back']) {
    final client = FakeServerClient(examGetReplies: const [answering2]);
    final saved = <List<String>>[];
    await tester.pumpWidget(MaterialApp(
      home: Builder(
        builder: (context) => TextButton(
          onPressed: () => Navigator.of(context).push(MaterialPageRoute(
            builder: (_) => ExamScreen(
              deckName: 'rust.md',
              client: client,
              support: tempSupport(),
              buildClient: (_) => client,
              saveAnswers: (_, answers) => saved.add(List.of(answers)),
              applyResult: (exam, nowMs) => null,
              applyRemediation: (_, _) => 0,
              nowMs: () => BigInt.from(1000),
              pollInterval: _pollInterval,
            ),
          )),
          child: const Text('open'),
        ),
      ),
    ));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    await tester.enterText(
      find.byKey(const ValueKey('exam-answer-field')),
      'ownership prevents aliasing writes',
    );

    switch (leave) {
      case 'Continue later':
        await tester.tap(find.text('Continue later'));
      case 'close button':
        await tester.tap(find.byIcon(Icons.close));
      default:
        await tester.binding.handlePopRoute();
    }
    await tester.pumpAndSettle();

    expect(find.byType(ExamScreen), findsNothing, reason: '$leave step 1: the screen is left');
    expect(
      saved,
      [
        ['ownership prevents aliasing writes', ''],
      ],
      reason: '$leave step 2: the answer visible when the learner pauses must be available on resume',
    );
    expect(client.examCloseCalls, 0, reason: '$leave step 3: the remote slot stays open');
    await tester.pumpWidget(const SizedBox.shrink());
    }
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

  testWidgets('a remediation the core fails to save shows its reason and can be retried',
      (tester) async {
    const reason = 'cannot save progress';
    final client = FakeServerClient(examGetReplies: const [remediated]);
    var failing = true;
    await pumpExam(
      tester,
      client: client,
      applyRemediation: (_, _) {
        if (failing) throw AnyhowException(reason);
        return 2;
      },
    );
    await tester.pump();

    expect(find.text(reason), findsOneWidget, reason: 'the failure is shown');
    expect(find.text('Try again'), findsOneWidget, reason: 'the remediation stays unapplied');

    failing = false;
    await tester.tap(find.text('Try again'));
    await tester.pumpAndSettle();
    await tester.pump(const Duration(seconds: 5));
    await tester.pumpAndSettle();

    expect(find.text('4 new cards to drill.'), findsOneWidget);
    expect(find.text('Try again'), findsNothing);
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
