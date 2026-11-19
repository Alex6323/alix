import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge.dart' show AnyhowException;
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bridge/exam_bridge.dart';
import 'package:alix_mobile/exam_screen.dart';
import 'package:alix_mobile/server_client.dart';

import 'support/fake_held_exam.dart';
import 'support/fake_server_client.dart';

const _pollInterval = Duration(milliseconds: 10);

const _needsDesktop = 'grading needs the paired desktop';
const _busy = 'the desktop is busy grading; try again shortly';
const _unreadable = 'the desktop could not read this exam; update alix on both devices';
const _changedDeck = 'the deck changed since this exam was prepared; its result cannot be recorded';
const _remediate = 'Turn the gaps into cards';
const _pairingLine = 'Pairing expired. Pair again from Settings → Connected devices.';

final _field = find.byKey(const ValueKey('exam-answer-field'));

GradingCall _answered(String phase, {String? sitting = 'sitting-1', String? error, int? elapsed}) {
  final json = <String, dynamic>{
    'sitting': sitting,
    'phase': phase,
    'thinking': phase == 'grading' || phase == 'remediating',
    'elapsed': elapsed,
    'error': error,
    'estimate': null,
    'grades': <Object>[],
    'topics': <Object>[],
    'verdicts': null,
    'mastered': false,
    'cards_by_topic': <Object>[],
  };
  return GradingAnswered(jsonEncode(json), RemoteGrading.fromJson(json)!);
}

String _body(GradingCall call) => (call as GradingAnswered).body;

FakeHeldExam _graded({
  List<String>? log,
  bool recorded = true,
  bool remediable = true,
  int remediationCount = 1,
  ({String text, bool examChanged})? standing,
  List<String> unsourcedTopics = const [],
  Map<String, dynamic>? gradedResult,
  List<String> questions = const ['Why ownership?', 'What is borrowing?'],
}) => FakeHeldExam(
  log: log,
  questions: questions,
  remediable: remediable,
  remediationCount: remediationCount,
  standing: standing,
  unsourcedTopics: unsourcedTopics,
  gradedResult: gradedResult,
)
  ..recorded = recorded
  ..outcomeRecorded = recorded;

class _ThrowingClient implements ServerClient {
  @override
  void close() {}

  @override
  dynamic noSuchMethod(Invocation invocation) => throw StateError('the desktop was called');
}

List<Object> _plain((List<String>, List<bool>) saved) => [saved.$1, saved.$2];

void main() {
  Widget screen({
    required HeldExamAccess held,
    ServerClient? client,
    VoidCallback? onLeaveResult,
    Future<void> Function()? onDiscard,
  }) => ExamScreen(
    deckName: 'rust.md',
    held: held,
    client: client,
    onLeaveResult: onLeaveResult ?? () {},
    onDiscard: onDiscard ?? () async {},
    pollInterval: _pollInterval,
  );

  Future<void> pumpExam(
    WidgetTester tester, {
    required HeldExamAccess held,
    ServerClient? client,
    VoidCallback? onLeaveResult,
  }) async {
    await tester.pumpWidget(MaterialApp(
      home: screen(held: held, client: client, onLeaveResult: onLeaveResult),
    ));
    await tester.pump();
  }

  Future<void> openExam(
    WidgetTester tester, {
    required HeldExamAccess held,
    ServerClient? client,
    VoidCallback? onLeaveResult,
    Future<void> Function()? onDiscard,
  }) async {
    await tester.pumpWidget(MaterialApp(
      home: Builder(
        builder: (context) => TextButton(
          onPressed: () => Navigator.of(context).push(MaterialPageRoute(
            builder: (_) => screen(held: held, client: client, onLeaveResult: onLeaveResult, onDiscard: onDiscard),
          )),
          child: const Text('open'),
        ),
      ),
    ));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
  }

  Future<void> answerAndSubmit(WidgetTester tester, {String first = 'a1', String second = 'a2'}) async {
    await tester.enterText(_field, first);
    await tester.tap(find.text('Next'));
    await tester.pump();
    await tester.enterText(_field, second);
    await tester.tap(find.text('Submit'));
    await tester.pump();
    await tester.pump();
  }

  bool remediateEnabled(WidgetTester tester) =>
      tester.widget<FilledButton>(find.widgetWithText(FilledButton, _remediate)).onPressed != null;

  testWidgets('the questions show and answers save with no client and with one that throws', (tester) async {
    for (final (label, client) in [('no client', null), ('a throwing client', _ThrowingClient())]) {
      final held = FakeHeldExam();
      await pumpExam(tester, held: held, client: client);

      expect(find.text('Why ownership?'), findsOneWidget, reason: '$label: question 1 shows');
      await tester.enterText(_field, 'a1');
      await tester.tap(find.text('Next'));
      await tester.pump();

      expect(find.text('What is borrowing?'), findsOneWidget, reason: '$label: Next shows question 2');
      expect(
        held.saved.map(_plain),
        [
          [['a1', ''], [true, false]],
        ],
        reason: '$label: Next saves the locked answer into the held file',
      );
      expect(tester.takeException(), isNull, reason: '$label: nothing throws');
      await tester.pumpWidget(const SizedBox.shrink());
    }
  });

  testWidgets('a lock over the limit shows the too-long line and the answer stays editable', (tester) async {
    final log = <String>[];
    final held = FakeHeldExam(log: log)
      ..tooLong = (answers, locked) => [
        for (var i = 0; i < answers.length; i++) locked[i] && answers[i].length > 10,
      ].contains(true);
    final client = FakeServerClient(callLog: log, gradingReplies: [_answered('grading')]);
    await pumpExam(tester, held: held, client: client);

    await tester.enterText(_field, 'far too long an answer');
    await tester.tap(find.text('Next'));
    await tester.pump();
    expect(find.text(answerTooLongText), findsOneWidget, reason: 'Next: the too-long line shows');
    expect(find.text('Question 1 of 2'), findsOneWidget, reason: 'Next: the pager stays on question 1');
    expect(tester.widget<TextField>(_field).readOnly, isFalse, reason: 'Next: the refused answer stays editable');

    await tester.enterText(_field, 'short');
    await tester.tap(find.text('Next'));
    await tester.pump();
    await tester.enterText(_field, 'also far too long');
    await tester.tap(find.text('Submit'));
    await tester.pump();
    expect(find.text(answerTooLongText), findsOneWidget, reason: 'Submit: the too-long line shows');
    expect(tester.widget<TextField>(_field).readOnly, isFalse, reason: 'Submit: the refused answer stays editable');
    expect(log.contains('gradeRequest') || log.contains('gradingSubmit'), isFalse,
        reason: 'Submit: a refused lock stops before the grade request, log $log');

    held
      ..tooLong = null
      ..gradeRequestTooLong = true;
    await tester.enterText(_field, 'fits');
    await tester.tap(find.text('Submit'));
    await tester.pump();
    expect(client.gradingRequests, isEmpty, reason: 'grade request: nothing is sent, log $log');
    expect(find.text(answerTooLongText), findsOneWidget, reason: 'grade request: the too-long line shows');
  });

  testWidgets('a changed deck is reported on open and on Submit before any client call; submit anyway stores not recorded',
      (tester) async {
    final log = <String>[];
    final held = FakeHeldExam(log: log, changed: true);
    final client = FakeServerClient(callLog: log, gradingReplies: [_answered('results')]);
    await pumpExam(tester, held: held, client: client);

    expect(find.text(_changedDeck), findsOneWidget, reason: 'open: the changed-deck line shows');
    expect(find.text('Discard exam'), findsOneWidget, reason: 'open: Discard exam is offered');
    expect(find.text('submit anyway'), findsOneWidget, reason: 'open: submit anyway is offered');
    expect(log, ['view', 'materialChanged'], reason: 'open: the material check follows the view, nothing is sent');

    log.clear();
    await answerAndSubmit(tester);
    expect(find.text(_changedDeck), findsOneWidget, reason: 'Submit: the changed-deck line shows');
    expect(log, ['saveAnswers', 'saveAnswers', 'materialChanged'],
        reason: 'Submit: the answers are saved and checked, nothing is sent');

    await tester.tap(find.text('submit anyway'));
    await tester.pump();
    await tester.pump();
    expect(held.stored.map((stored) => stored.$2), [false], reason: 'submit anyway: the result is stored not recorded');
    expect(find.text('not recorded'), findsOneWidget, reason: 'submit anyway: the result says not recorded');

    await tester.pumpWidget(const SizedBox.shrink());
    final checkedLater = FakeHeldExam();
    final laterClient = FakeServerClient(gradingReplies: [_answered('results')]);
    await pumpExam(tester, held: checkedLater, client: laterClient);
    expect(find.text(_changedDeck), findsNothing, reason: 'unchanged at open: no line');
    checkedLater.changed = true;
    await answerAndSubmit(tester);
    expect(find.text(_changedDeck), findsOneWidget, reason: 'changed by Submit: the line shows');
    expect(laterClient.gradingRequests, isEmpty, reason: 'changed by Submit: nothing is sent');
  });

  testWidgets('no answer, 409 and 400 each show their line and store nothing', (tester) async {
    for (final (label, client, line) in [
      ('no client', null, _needsDesktop),
      ('no answer', FakeServerClient(gradingReplies: const [GradingNoAnswer()]), _needsDesktop),
      ('409', FakeServerClient(gradingReplies: const [GradingRefused(409)]), _busy),
      ('400', FakeServerClient(gradingReplies: const [GradingRefused(400)]), _unreadable),
    ]) {
      final held = FakeHeldExam();
      await pumpExam(tester, held: held, client: client);
      await answerAndSubmit(tester);

      expect(find.text(line), findsOneWidget, reason: '$label: the line "$line" shows');
      expect(held.stored, isEmpty, reason: '$label: nothing is stored');
      expect(find.text('Question 2 of 2'), findsOneWidget, reason: '$label: the answers stay');
      await tester.pumpWidget(const SizedBox.shrink());
    }
  });

  testWidgets('a refused status other than 400 and 409 shows the status line', (tester) async {
    final held = FakeHeldExam();
    await pumpExam(tester, held: held, client: FakeServerClient(gradingReplies: const [GradingRefused(503)]));
    await answerAndSubmit(tester);

    expect(find.text('the desktop refused this request (503)'), findsOneWidget, reason: '503: the status line shows');
    expect(held.stored, isEmpty, reason: '503: nothing is stored');
  });

  testWidgets('a failed job shows its error with submit again', (tester) async {
    final held = FakeHeldExam();
    final client = FakeServerClient(
      gradingReplies: [_answered('grading'), _answered('grading', error: 'the backend failed')],
    );
    await pumpExam(tester, held: held, client: client);
    await answerAndSubmit(tester);
    await tester.pump(_pollInterval);
    await tester.pump();

    expect(find.text('the backend failed'), findsOneWidget, reason: 'job error: its text shows');
    expect(find.text('submit again'), findsOneWidget, reason: 'job error: submit again is offered');
    expect(held.stored, isEmpty, reason: 'job error: nothing is stored');
  });

  testWidgets('after an accepted grade or remediation, a poll showing idle or another sitting returns the screen',
      (tester) async {
    for (final (label, gone) in [
      ('idle', _answered('idle', sitting: null)),
      ('another sitting', _answered('grading', sitting: 'sitting-2')),
    ]) {
      final held = FakeHeldExam();
      await pumpExam(tester, held: held, client: FakeServerClient(gradingReplies: [_answered('grading'), gone]));
      await answerAndSubmit(tester);
      expect(find.textContaining('The server is working'), findsOneWidget, reason: 'grade $label: accepted, working');
      await tester.pump(_pollInterval);
      await tester.pump();

      expect(find.text('Question 2 of 2'), findsOneWidget, reason: 'grade $label: back to the answers');
      expect(find.text('submit again'), findsOneWidget, reason: 'grade $label: submit again is offered');
      expect(held.stored, isEmpty, reason: 'grade $label: nothing is stored');
      await tester.pumpWidget(const SizedBox.shrink());
    }
    for (final (label, gone) in [
      ('idle', _answered('idle', sitting: null)),
      ('another sitting', _answered('remediating', sitting: 'sitting-2')),
    ]) {
      final held = _graded();
      await pumpExam(tester, held: held, client: FakeServerClient(gradingReplies: [_answered('remediating'), gone]));
      await tester.tap(find.text(_remediate));
      await tester.pump();
      await tester.pump(_pollInterval);
      await tester.pump();

      expect(find.text('Ownership'), findsOneWidget, reason: 'remediate $label: the result stays');
      expect(find.text('remediate again'), findsOneWidget, reason: 'remediate $label: remediate again is offered');
      expect(held.storedRemediations, isEmpty, reason: 'remediate $label: nothing is stored');
      await tester.pumpWidget(const SizedBox.shrink());
    }
  });

  testWidgets('a poll tick with no answer stops polling with the needs-desktop line and submit again', (tester) async {
    final log = <String>[];
    final held = FakeHeldExam(log: log);
    await pumpExam(
      tester,
      held: held,
      client: FakeServerClient(callLog: log, gradingReplies: [_answered('grading'), const GradingNoAnswer()]),
    );
    await answerAndSubmit(tester);
    for (var tick = 0; tick < 4; tick++) {
      await tester.pump(_pollInterval);
    }

    expect(find.text(_needsDesktop), findsOneWidget, reason: 'poll no answer: the needs-desktop line shows');
    expect(find.text('submit again'), findsOneWidget, reason: 'poll no answer: submit again is offered');
    expect(log.where((call) => call == 'gradingGet'), hasLength(1), reason: 'poll no answer: polling stops, log $log');
  });

  testWidgets('an expired pairing on Submit and on Remediate shows the pairing line, stores nothing and keeps the screen',
      (tester) async {
    final answering = FakeHeldExam();
    await openExam(tester, held: answering, client: FakeServerClient(expireOnGrading: true));
    await answerAndSubmit(tester);
    expect(find.text(_pairingLine), findsOneWidget, reason: 'Submit: the pairing line shows');
    expect(answering.stored, isEmpty, reason: 'Submit: nothing is stored');
    expect(find.byType(ExamScreen), findsOneWidget, reason: 'Submit: the screen stays');
    await tester.pumpWidget(const SizedBox.shrink());

    final graded = _graded();
    await openExam(tester, held: graded, client: FakeServerClient(expireOnGrading: true));
    await tester.tap(find.text(_remediate));
    await tester.pump();
    await tester.pump();
    expect(find.text(_pairingLine), findsOneWidget, reason: 'Remediate: the pairing line shows');
    expect(graded.storedRemediations, isEmpty, reason: 'Remediate: nothing is stored');
    expect(find.byType(ExamScreen), findsOneWidget, reason: 'Remediate: the screen stays');
  });

  testWidgets('reopening after a kill during grading submits byte-equal answers and stores the first grading',
      (tester) async {
    final firstLog = <String>[];
    final held = FakeHeldExam();
    final first = FakeServerClient(callLog: firstLog, gradingReplies: [_answered('grading')]);
    await pumpExam(tester, held: held, client: first);
    await answerAndSubmit(tester);
    expect(find.textContaining('The server is working'), findsOneWidget, reason: 'first: the grading is running');
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pump(_pollInterval * 3);
    expect(firstLog, ['gradingSubmit'], reason: 'killed: no poll outlives the screen');

    final results = _answered('results');
    final second = FakeServerClient(gradingReplies: [results]);
    await pumpExam(tester, held: held, client: second);
    expect(find.text('Question 2 of 2'), findsOneWidget, reason: 'reopened: every answer is locked, the last shows');
    await tester.tap(find.text('Submit'));
    await tester.pump();
    await tester.pump();

    expect(second.gradingRequests.single, first.gradingRequests.single,
        reason: 'reopened: the request is byte-equal to the first');
    expect(held.stored.map((stored) => stored.$1), [_body(results)], reason: 'reopened: the grading is stored once');
  });

  testWidgets('a lost reply, then a blank answer filled: the stored result grades the filled answer', (tester) async {
    final held = FakeHeldExam();
    final client = FakeServerClient(gradingReplies: [const GradingNoAnswer(), _answered('results')]);
    await pumpExam(tester, held: held, client: client);
    await answerAndSubmit(tester, second: '');
    expect(find.text(_needsDesktop), findsOneWidget, reason: 'lost reply: the needs-desktop line shows');
    expect(tester.widget<TextField>(_field).readOnly, isFalse, reason: 'lost reply: the blank answer stays editable');

    await tester.enterText(_field, 'filled later');
    await tester.tap(find.text('submit again'));
    await tester.pump();
    await tester.pump();

    expect(
      [for (final request in client.gradingRequests) (jsonDecode(request) as Map)['answers']],
      [
        ['a1', ''],
        ['a1', 'filled later'],
      ],
      reason: 'the second request carries the answer on screen at Submit',
    );
    expect(held.stored, hasLength(1), reason: 'one result is stored');
    expect(held.view()!.result!.grades.map((grade) => grade.answer), ['a1', 'filled later'],
        reason: 'the stored result grades the filled answer');
  });

  testWidgets('a result is stored, then the desk is closed, then it is applied; a failed apply offers try again',
      (tester) async {
    final log = <String>[];
    final closeGate = Completer<void>();
    addTearDown(() {
      if (!closeGate.isCompleted) closeGate.complete();
    });
    final held = FakeHeldExam(log: log, applyFailures: 1);
    final client = FakeServerClient(
      callLog: log,
      gradingReplies: [_answered('results')],
      gradingCloseGate: closeGate,
    );
    await pumpExam(tester, held: held, client: client);
    await answerAndSubmit(tester);

    expect(log.sublist(log.indexOf('gradingSubmit')), ['gradingSubmit', 'storeResult', 'gradingClose', 'apply'],
        reason: 'store, then close, then apply, log $log');
    expect(closeGate.isCompleted, isFalse, reason: 'the apply ran while the close was still pending');
    expect(client.gradingClosedSittings, ['sitting-1'], reason: 'the close names the sitting');
    expect(find.text('Bad state: cannot write the progress folder'), findsOneWidget,
        reason: 'failed apply: its text shows');
    expect(find.text('try again'), findsOneWidget, reason: 'failed apply: try again is offered');

    await tester.tap(find.text('try again'));
    await tester.pump();
    expect(log.where((call) => call == 'apply'), hasLength(2), reason: 'try again: apply runs again');
    expect(find.text('try again'), findsNothing, reason: 'try again: the result shows');
    expect(find.text('Ownership'), findsOneWidget, reason: 'try again: the topic of the result shows');
  });

  testWidgets('a storeResult that throws sends no close and applies nothing', (tester) async {
    final log = <String>[];
    final held = FakeHeldExam(log: log)..storeError = AnyhowException('the reply does not fit the exam');
    final client = FakeServerClient(callLog: log, gradingReplies: [_answered('results')]);
    await pumpExam(tester, held: held, client: client);
    await answerAndSubmit(tester);

    expect(log.contains('gradingClose') || log.contains('apply'), isFalse,
        reason: 'store refused: no close and no apply, log $log');
    expect(find.text('the reply does not fit the exam'), findsOneWidget, reason: 'store refused: its text shows');
    expect(find.text('submit again'), findsOneWidget, reason: 'store refused: submit again is offered');
  });

  testWidgets('leaving the answers sends nothing; leaving a result with an outcome reports it once', (tester) async {
    final log = <String>[];
    final answering = FakeHeldExam(log: log);
    var leaves = 0;
    await openExam(
      tester,
      held: answering,
      client: FakeServerClient(callLog: log),
      onLeaveResult: () => leaves++,
    );
    await tester.enterText(_field, 'half an answer');
    await tester.tap(find.text('Continue later'));
    await tester.pumpAndSettle();
    expect(find.byType(ExamScreen), findsNothing, reason: 'answers: the screen is left');
    expect(log, ['view', 'materialChanged', 'saveAnswers'], reason: 'answers: the draft is saved and nothing is sent');
    expect(leaves, 0, reason: 'answers: no result is reported');

    final graded = _graded();
    await openExam(tester, held: graded, client: FakeServerClient(), onLeaveResult: () => leaves++);
    await tester.tap(find.byIcon(Icons.close));
    await tester.pumpAndSettle();
    expect(find.byType(ExamScreen), findsNothing, reason: 'result: the screen is left');
    expect(leaves, 1, reason: 'result with an outcome: reported once');
    expect(graded.log.contains('remove'), isFalse, reason: 'result: the screen never removes the file itself');

    final unapplied = FakeHeldExam(applyFailures: 9)..recorded = true;
    await openExam(tester, held: unapplied, client: FakeServerClient(), onLeaveResult: () => leaves++);
    expect(find.text('try again'), findsOneWidget, reason: 'apply failed: try again is offered');
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(find.byType(ExamScreen), findsNothing, reason: 'apply failed: the screen is left');
    expect(leaves, 1, reason: 'apply failed: leaving reports nothing');
  });

  testWidgets('a reply after the screen is gone stores nothing', (tester) async {
    final gate = Completer<GradingCall>();
    final held = FakeHeldExam();
    await pumpExam(tester, held: held, client: FakeServerClient(gradingGates: {0: gate}));
    await answerAndSubmit(tester);
    await tester.pumpWidget(const SizedBox.shrink());

    gate.complete(_answered('results'));
    await tester.pump();
    expect(held.stored, isEmpty, reason: 'a late results reply touches nothing');
    expect(tester.takeException(), isNull, reason: 'a late reply throws nothing');
  });

  testWidgets('killed on the result: reopening shows it and Remediate still works', (tester) async {
    final held = FakeHeldExam(remediable: true, remediationCount: 2);
    await pumpExam(tester, held: held, client: FakeServerClient(gradingReplies: [_answered('results')]));
    await answerAndSubmit(tester);
    expect(find.text(_remediate), findsOneWidget, reason: 'first: the result offers Remediate');
    await tester.pumpWidget(const SizedBox.shrink());

    final remediated = _answered('remediated');
    final client = FakeServerClient(gradingReplies: [_answered('remediating'), remediated]);
    await pumpExam(tester, held: held, client: client);
    expect(find.text('Ownership'), findsOneWidget, reason: 'reopened: the result shows');
    expect(held.log.where((call) => call == 'apply'), hasLength(1), reason: 'reopened: the outcome is not applied again');
    expect(remediateEnabled(tester), isTrue, reason: 'reopened: Remediate is enabled at once');

    await tester.tap(find.text(_remediate));
    await tester.pump();
    await tester.pump(_pollInterval);
    await tester.pump();

    expect(client.remediateRequests, hasLength(1), reason: 'remediate: one request is sent');
    expect(held.storedRemediations, [_body(remediated)], reason: 'remediate: the reply is stored');
    expect(client.gradingClosedSittings, ['sitting-1'], reason: 'remediate: the desk is closed after the store');
    expect(find.text('2 new cards to drill.'), findsOneWidget, reason: 'remediate: the snack names the sum');
    expect(find.text(_remediate), findsNothing, reason: 'remediate: the file has nothing left to remediate');
  });

  testWidgets('a results reply naming another sitting is not stored', (tester) async {
    final held = FakeHeldExam();
    await pumpExam(
      tester,
      held: held,
      client: FakeServerClient(gradingReplies: [_answered('results', sitting: 'sitting-2')]),
    );
    await answerAndSubmit(tester);

    expect(held.stored, isEmpty, reason: 'another sitting: nothing is stored');
    expect(find.text('submit again'), findsOneWidget, reason: 'another sitting: submit again is offered');
  });

  testWidgets('a result that was not recorded, and one whose apply failed, offer no Remediate', (tester) async {
    final notRecorded = _graded(recorded: false);
    await pumpExam(tester, held: notRecorded, client: FakeServerClient());
    expect(find.text('not recorded'), findsOneWidget, reason: 'not recorded: the result says so');
    expect(find.text(_remediate), findsNothing, reason: 'not recorded: no Remediate');
    await tester.pumpWidget(const SizedBox.shrink());

    final unapplied = FakeHeldExam(applyFailures: 1, remediable: true)..recorded = true;
    await pumpExam(tester, held: unapplied, client: FakeServerClient());
    expect(find.text('try again'), findsOneWidget, reason: 'apply failed: try again is offered');
    expect(find.text(_remediate), findsNothing, reason: 'apply failed: no Remediate');
  });

  testWidgets('Remediate is not sent before the grading close has returned', (tester) async {
    for (final (label, closeReply) in [('returned', true), ('failed', false)]) {
      final log = <String>[];
      final closeGate = Completer<void>();
      final held = FakeHeldExam(log: log, remediable: true);
      final client = FakeServerClient(
        callLog: log,
        gradingReplies: [_answered('results'), _answered('remediating')],
        gradingCloseGate: closeGate,
        gradingCloseReply: closeReply,
      );
      await pumpExam(tester, held: held, client: client);
      await answerAndSubmit(tester);

      expect(remediateEnabled(tester), isFalse, reason: '$label: Remediate waits for the close');
      await tester.tap(find.text(_remediate), warnIfMissed: false);
      await tester.pump();
      expect(client.remediateRequests, isEmpty, reason: '$label: nothing is sent while the close is pending');

      closeGate.complete();
      await tester.pump();
      await tester.pump();
      expect(remediateEnabled(tester), isTrue, reason: '$label: the settled close enables Remediate');
      await tester.tap(find.text(_remediate));
      await tester.pump();
      expect(log.sublist(log.indexOf('gradingClose')), ['gradingClose', 'apply', 'remediationRequest', 'gradingRemediate'],
          reason: '$label: the remediation follows the close, log $log');
      await tester.pumpWidget(const SizedBox.shrink());
    }
  });

  testWidgets('a storeRemediation that throws shows its text with remediate again and sends no close', (tester) async {
    final held = _graded()..remediationError = AnyhowException('cannot save progress');
    final client = FakeServerClient(gradingReplies: [_answered('remediated')]);
    await pumpExam(tester, held: held, client: client);
    await tester.tap(find.text(_remediate));
    await tester.pump();
    await tester.pump();

    expect(find.text('cannot save progress'), findsOneWidget, reason: 'store refused: its text shows');
    expect(find.text('remediate again'), findsOneWidget, reason: 'store refused: remediate again is offered');
    expect(client.gradingClosedSittings, isEmpty, reason: 'store refused: no close is sent');
  });

  testWidgets('the remediation snack names the sum and is absent for a sum of zero', (tester) async {
    for (final (count, snack) in [(0, '0 new cards to drill.'), (3, '3 new cards to drill.')]) {
      final held = _graded(remediationCount: count);
      await pumpExam(tester, held: held, client: FakeServerClient(gradingReplies: [_answered('remediated')]));
      await tester.tap(find.text(_remediate));
      await tester.pump();
      await tester.pump();

      expect(held.storedRemediations, hasLength(1), reason: 'sum $count: the reply is stored');
      expect(find.text(snack), count == 0 ? findsNothing : findsOneWidget, reason: 'sum $count: the snack "$snack"');
      await tester.pumpWidget(const SizedBox.shrink());
    }
  });

  testWidgets('Discard exam on the changed-deck line calls the discard callback once and leaves', (tester) async {
    final discarded = Completer<void>();
    var discards = 0;
    final held = FakeHeldExam(changed: true);
    await openExam(
      tester,
      held: held,
      client: FakeServerClient(),
      onDiscard: () {
        discards++;
        return discarded.future;
      },
    );
    await tester.tap(find.text('Discard exam'));
    await tester.pump();
    expect(discards, 1, reason: 'Discard exam: the callback is called');
    expect(find.byType(ExamScreen), findsOneWidget, reason: 'Discard exam: the screen waits for the discard');

    discarded.complete();
    await tester.pumpAndSettle();
    expect(find.byType(ExamScreen), findsNothing, reason: 'Discard exam: the screen is left after the discard');
    expect(discards, 1, reason: 'Discard exam: called once');
    expect(held.log.contains('remove'), isFalse, reason: 'Discard exam: the screen never removes the file itself');
  });

  testWidgets('an exam whose file is gone leaves at once', (tester) async {
    await openExam(tester, held: FakeHeldExam()..gone = true, client: FakeServerClient());

    expect(find.byType(ExamScreen), findsNothing, reason: 'no file: the screen is left');
    expect(find.text('open'), findsOneWidget, reason: 'no file: the opener shows again');
  });

  testWidgets('a file that cannot be read shows its error and offers nothing', (tester) async {
    await pumpExam(tester, held: FakeHeldExam()..viewError = AnyhowException('unreadable held exam'));

    expect(find.text('unreadable held exam'), findsOneWidget, reason: 'unreadable: its text shows');
    expect(find.byType(FilledButton), findsNothing, reason: 'unreadable: no action is offered');
    expect(find.byType(OutlinedButton), findsNothing, reason: 'unreadable: no action is offered');
  });

  testWidgets('a material check that throws at Submit shows its text and sends nothing', (tester) async {
    final held = FakeHeldExam();
    final client = FakeServerClient(gradingReplies: [_answered('results')]);
    await pumpExam(tester, held: held, client: client);
    held.materialError = AnyhowException('the deck cannot be read');
    await answerAndSubmit(tester);

    expect(find.text('the deck cannot be read'), findsOneWidget, reason: 'check failed: its text shows');
    expect(client.gradingRequests, isEmpty, reason: 'check failed: nothing is sent');
  });

  testWidgets('at phone size the changed-deck line and a result with standing and Remediate lay out without overflow',
      (tester) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);

    await pumpExam(tester, held: FakeHeldExam(changed: true), client: FakeServerClient());
    expect(find.text(_changedDeck), findsOneWidget, reason: 'answers: the changed-deck line shows');
    expect(tester.takeException(), isNull, reason: 'answers with the changed-deck line: no overflow');

    await tester.pumpWidget(const SizedBox.shrink());
    await pumpExam(
      tester,
      held: _graded(standing: (text: 'passed (4 Nov)', examChanged: false)),
      client: FakeServerClient(),
    );
    expect(find.text('passed (4 Nov)'), findsOneWidget, reason: 'result: the standing shows');
    expect(find.text(_remediate), findsOneWidget, reason: 'result: Remediate shows');
    expect(tester.takeException(), isNull, reason: 'result with standing and Remediate: no overflow');
  });

  testWidgets('a changed mark underlines dashed and unsourced topics list on the results', (tester) async {
    await pumpExam(
      tester,
      held: _graded(standing: (text: 'mastered (4 Nov)', examChanged: true), unsourcedTopics: const ['Terms', 'Intro']),
    );

    final mark = tester.widget<Text>(find.text('mastered (4 Nov)'));
    expect(mark.style?.decoration, TextDecoration.underline, reason: 'changed: the standing is underlined');
    expect(mark.style?.decorationStyle, TextDecorationStyle.dashed, reason: 'changed: the underline is dashed');
    expect(find.text('Not examined: Terms, Intro'), findsOneWidget, reason: 'the unsourced topics are listed');
  });

  testWidgets('results show a per-topic count and mark', (tester) async {
    await pumpExam(tester, held: _graded());

    expect(find.text('Ownership'), findsOneWidget, reason: 'the topic label shows');
    expect(find.text('needs work'), findsOneWidget, reason: 'a failed topic reads needs work');
    expect(find.text('0 / 2'), findsWidgets, reason: 'the topic count shows');
    expect(find.text('pass'), findsNothing, reason: 'no topic passed');
  });

  testWidgets('no result wording says passed or failed', (tester) async {
    for (final held in [_graded(), _graded(recorded: false)]) {
      await pumpExam(tester, held: held);
      expect(find.textContaining(RegExp('passed|failed', caseSensitive: false)), findsNothing,
          reason: 'recorded ${held.recorded}: no passed or failed wording');
      await tester.pumpWidget(const SizedBox.shrink());
    }
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
    final held = FakeHeldExam(
      questions: [
        'A committee of four validators holds this stake out of a total voting power of 10,000:\n\n'
            '```\nvalidator A: 4,000\nvalidator B: 3,000\nvalidator C: 2,000\nvalidator D: 1,000\n```\n\n$prose',
      ],
      questionsUnits: [
        [
          _sentence('A committee of four validators holds this stake out of a total voting power of 10,000:'),
          _code(['validator A: 4,000', 'validator B: 3,000', 'validator C: 2,000', 'validator D: 1,000']),
          _sentence(prose),
        ],
      ],
    );
    await pumpExam(tester, held: held);

    expect(find.textContaining('```'), findsNothing, reason: 'the question renders its units, not raw markdown');
    final screenSize = tester.view.physicalSize / tester.view.devicePixelRatio;
    final keyboardTop = screenSize.height - tester.view.viewInsets.bottom / tester.view.devicePixelRatio;
    final field = tester.getRect(_field);
    expect(field.height, greaterThanOrEqualTo(96), reason: 'answer field rect $field must keep typing room');
    expect(field.bottom, lessThanOrEqualTo(keyboardTop),
        reason: 'answer field rect $field must stay above keyboard top $keyboardTop');
    expect(tester.takeException(), isNull, reason: 'no layout overflow');
  });

  FakeHeldExam longQuestions(int count) {
    final prose = List.filled(40, 'This question is long enough to scroll on a phone.').join(' ');
    return FakeHeldExam(
      questions: [for (var i = 1; i <= count; i++) 'Start of question $i. $prose'],
      questionsUnits: [
        for (var i = 1; i <= count; i++) [_sentence('Start of question $i. $prose')],
      ],
    );
  }

  testWidgets('a long question shows a scrollbar and a hairline above the answer', (tester) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);
    await pumpExam(tester, held: longQuestions(1));

    final scrollbar = tester.widget<Scrollbar>(
      find.ancestor(of: find.byKey(const ValueKey('exam-question')), matching: find.byType(Scrollbar)),
    );
    expect(scrollbar.thumbVisibility, isTrue, reason: 'the scrollbar shows without a touch');
    final question = tester.getRect(find.byKey(const ValueKey('exam-question')));
    final hairline = tester.getRect(find.byKey(const ValueKey('exam-question-hairline')));
    final field = tester.getRect(_field);
    expect(hairline.top, greaterThanOrEqualTo(question.bottom), reason: 'hairline $hairline below question $question');
    expect(hairline.bottom, lessThanOrEqualTo(field.top), reason: 'hairline $hairline above field $field');
  });

  testWidgets('the next question starts at its top, not at the last scroll offset', (tester) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);
    await pumpExam(tester, held: longQuestions(2));

    await tester.drag(find.byKey(const ValueKey('exam-question')), const Offset(0, -300));
    await tester.pumpAndSettle();
    expect(find.textContaining('Start of question 1'), findsOneWidget, reason: 'question 1 shows');
    await tester.enterText(_field, 'an answer');
    await tester.tap(find.text('Next'));
    await tester.pumpAndSettle();

    final question = tester.getRect(find.byKey(const ValueKey('exam-question')));
    final start = tester.getRect(find.textContaining('Start of question 2'));
    expect(start.top, greaterThanOrEqualTo(question.top),
        reason: 'question 2 text $start must begin inside the question area $question');
  });

  testWidgets('an exam code block numbers its lines so a question can cite them', (tester) async {
    final lines = ['fn main() {', '    let x;', '    x = 1;', '}'];
    await pumpExam(
      tester,
      held: FakeHeldExam(
        questions: const ['What does line 3 do?\n\n```\nfn main() {\n    let x;\n    x = 1;\n}\n```'],
        questionsUnits: [
          [_sentence('What does line 3 do?'), _code(lines)],
        ],
      ),
    );

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
    await pumpExam(
      tester,
      held: FakeHeldExam(
        questions: const ['What does line 100 do?'],
        questionsUnits: [
          [
            _code([for (var i = 1; i <= 100; i++) 'line $i']),
          ],
        ],
      ),
    );

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
    await pumpExam(
      tester,
      held: FakeHeldExam(
        questions: const ['What do these lines do?'],
        questionsUnits: [
          [
            _code([wrapped, 'short second line']),
          ],
        ],
      ),
    );

    final firstNumber = tester.getRect(find.text('1'));
    final firstLine = tester.getRect(find.text(wrapped));
    final secondNumber = tester.getRect(find.text('2'));
    final secondLine = tester.getRect(find.text('short second line'));
    expect(firstNumber.top, firstLine.top, reason: 'line 1 starts beside its number');
    expect(firstLine.height, greaterThan(secondLine.height), reason: 'line 1 actually wraps');
    expect(secondNumber.top, greaterThanOrEqualTo(firstLine.bottom), reason: 'line 2 follows all of line 1');
    expect(secondNumber.top, secondLine.top, reason: 'line 2 starts beside its number');
    expect(tester.takeException(), isNull, reason: 'no overflow');
  });

  Map<String, dynamic> oneGrade(Map<String, dynamic> grade, {int pass = 0, int partial = 1}) => {
    'grades': [grade],
    'topics': [
      {
        'topic_id': 'topic-a',
        'fingerprint': 'fp-a',
        'label': 'Stake',
        'cards': ['card-a'],
        'pass': pass == 1,
        'verdicts': {'pass': pass, 'partial': partial, 'total': 1},
      },
    ],
    'verdicts': {'pass': pass, 'partial': partial, 'total': 1},
    'mastered': false,
  };

  testWidgets('results identify each grade with its rendered question', (tester) async {
    const question = 'Which validator has the largest stake?\n\n```\nvalidator A: 4,000\nvalidator B: 3,000\n```';
    await pumpExam(
      tester,
      held: _graded(
        questions: const [question],
        gradedResult: oneGrade({
          'question': question,
          'question_units': [
            _sentence('Which validator has the largest stake?'),
            _code(['validator A: 4,000', 'validator B: 3,000']),
          ],
          'points': ['validator A'],
          'answer': 'A',
          'verdict': 'PARTIAL',
          'feedback': 'Name the stake as well.',
          'missed': ['4,000'],
        }),
      ),
    );

    expect(find.text('Which validator has the largest stake?'), findsOneWidget,
        reason: 'each verdict must identify the question it grades');
    expect(find.text('validator A: 4,000'), findsOneWidget,
        reason: 'the grade question must use the content units');
    expect(find.text('validator B: 3,000'), findsOneWidget,
        reason: 'the grade question must use the content units');
    expect(find.textContaining('```'), findsNothing, reason: 'results must not expose raw Markdown fences');
  });

  testWidgets('results render structured feedback and failed rubric details', (tester) async {
    await pumpExam(
      tester,
      held: _graded(
        questions: const ['Which validator has the largest stake?'],
        gradedResult: oneGrade({
          'question': 'Which validator has the largest stake?',
          'question_units': [_sentence('Which validator has the largest stake?')],
          'points': ['Name the validator and stake.'],
          'points_units': [
            [
              _code(['complete: validator A has 4,000']),
            ],
          ],
          'answer': 'A',
          'verdict': 'PARTIAL',
          'feedback': 'Almost.\n\n```\nfeedback: include 4,000\n```',
          'feedback_units': [
            _sentence('Almost.'),
            _code(['feedback: include 4,000']),
          ],
          'missed': ['4,000'],
          'missed_units': [
            [
              _code(['missed: 4,000']),
            ],
          ],
        }),
      ),
    );

    expect(find.text('feedback: include 4,000'), findsOneWidget, reason: 'feedback must use feedback_units');
    expect(find.text('A complete answer covers:'), findsOneWidget, reason: 'a partial verdict must show its rubric points');
    expect(find.text('complete: validator A has 4,000'), findsOneWidget, reason: 'rubric points must use points_units');
    expect(find.text('You missed:'), findsOneWidget, reason: 'a partial verdict must name its missed points');
    expect(find.text('missed: 4,000'), findsOneWidget, reason: 'missed points must use missed_units');
    expect(find.textContaining('```'), findsNothing, reason: 'results must not expose raw Markdown fences');
  });

  testWidgets('a result identifies its question before announcing the verdict', (tester) async {
    const question = 'Which validator has the largest stake?';
    await pumpExam(
      tester,
      held: _graded(
        questions: const [question],
        gradedResult: oneGrade({
          'question': question,
          'points': ['Name the validator and stake.'],
          'answer': 'A',
          'verdict': 'PARTIAL',
          'feedback': 'Almost.',
          'missed': ['4,000'],
          'shape': 'boundary',
        }),
      ),
    );

    final questionTop = tester.getTopLeft(find.text(question)).dy;
    final verdictTop = tester.getTopLeft(find.text('PARTIAL')).dy;
    expect(questionTop, lessThan(verdictTop),
        reason: 'question top $questionTop must come before verdict top $verdictTop');
    expect(find.text('boundary'), findsOneWidget, reason: 'the question shape shows');
  });

  testWidgets('short grade unit lists fall back at the matching raw index', (tester) async {
    await pumpExam(
      tester,
      held: _graded(
        questions: const ['Why?'],
        gradedResult: oneGrade({
          'question': 'Why?',
          'points': ['first raw point', 'second raw point'],
          'points_units': [
            [
              _code(['first structured point']),
            ],
          ],
          'answer': 'Because.',
          'verdict': 'PARTIAL',
          'feedback': 'Almost.',
          'missed': ['raw missed point'],
          'missed_units': <Object>[],
        }),
      ),
    );

    expect(find.text('first structured point'), findsOneWidget, reason: 'point 1 uses its units');
    expect(find.text('first raw point'), findsNothing, reason: 'point 1 raw text is replaced');
    expect(find.text('second raw point'), findsOneWidget, reason: 'point 2 falls back to raw text');
    expect(find.text('raw missed point'), findsOneWidget, reason: 'the missed point falls back to raw text');
  });

  testWidgets('a pass formats feedback without exposing rubric details', (tester) async {
    await pumpExam(
      tester,
      held: _graded(
        questions: const ['Why?'],
        gradedResult: oneGrade({
          'question': 'Why?',
          'points': ['A complete point.'],
          'answer': 'Because.',
          'verdict': 'PASS',
          'feedback': '```\nraw feedback\n```',
          'feedback_units': [
            _code(['structured feedback']),
          ],
          'missed': <Object>[],
        }, pass: 1, partial: 0),
      ),
    );

    expect(find.text('structured feedback'), findsOneWidget, reason: 'feedback uses its units');
    expect(find.textContaining('```'), findsNothing, reason: 'no raw Markdown fences');
    expect(find.text('A complete answer covers:'), findsNothing, reason: 'a pass shows no rubric points');
    expect(find.text('You missed:'), findsNothing, reason: 'a pass shows no missed points');
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
    await pumpExam(
      tester,
      held: _graded(
        questions: questions,
        gradedResult: {
          'grades': [
            for (final question in questions)
              {
                'question': question,
                'question_units': [_sentence(question)],
                'points': <Object>[],
                'answer': 'Answer',
                'verdict': 'PASS',
                'feedback': 'Correct.',
                'missed': <Object>[],
              },
          ],
          'topics': <Object>[],
          'verdicts': {'pass': 8, 'partial': 0, 'total': 8},
          'mastered': true,
        },
      ),
    );

    await tester.scrollUntilVisible(find.text(questions.last), 300, scrollable: find.byType(Scrollable).first);
    expect(find.text(questions.last), findsOneWidget, reason: 'the last question can be scrolled to');
    expect(tester.takeException(), isNull, reason: 'long result rows must scroll without overflow');
  });

  testWidgets('answering navigation keeps typed text across Back/Next', (tester) async {
    final held = FakeHeldExam();
    final client = FakeServerClient(gradingReplies: [_answered('grading')]);
    await pumpExam(tester, held: held, client: client);

    expect(find.text('Question 1 of 2'), findsOneWidget, reason: 'opens on question 1');
    await tester.enterText(_field, 'a1');
    await tester.tap(find.text('Next'));
    await tester.pump();
    expect(find.text('Question 2 of 2'), findsOneWidget, reason: 'Next shows question 2');
    await tester.enterText(_field, 'a2');
    await tester.tap(find.text('Back'));
    await tester.pump();

    expect(find.text('Question 1 of 2'), findsOneWidget, reason: 'Back shows question 1');
    expect(tester.widget<TextField>(_field).controller?.text, 'a1', reason: 'question 1 keeps its own text');

    await tester.tap(find.text('Next'));
    await tester.pump();
    await tester.tap(find.text('Submit'));
    await tester.pump();
    expect((jsonDecode(client.gradingRequests.single) as Map)['answers'], ['a1', 'a2'],
        reason: 'Submit sends both answers as typed');
  });

  testWidgets('moving on persists and locks an answer, and reopening starts at the first empty one', (tester) async {
    final held = FakeHeldExam();
    await pumpExam(tester, held: held);

    await tester.enterText(_field, 'first answer');
    await tester.tap(find.text('Next'));
    await tester.pump();
    expect(held.saved.map(_plain), [
      [['first answer', ''], [true, false]],
    ], reason: 'Next saves the answer locked');
    await tester.tap(find.text('Back'));
    await tester.pump();
    expect(tester.widget<TextField>(_field).readOnly, isTrue, reason: 'a locked answer is read only');

    await tester.pumpWidget(const SizedBox.shrink());
    await pumpExam(tester, held: FakeHeldExam(answers: ['first answer', ''], locked: [true, false]));

    expect(find.text('Question 2 of 2'), findsOneWidget, reason: 'reopened: the first open answer shows');
    await tester.tap(find.text('Back'));
    await tester.pump();
    final resumed = tester.widget<TextField>(_field);
    expect(resumed.controller?.text, 'first answer', reason: 'reopened: the saved answer is back');
    expect(resumed.readOnly, isTrue, reason: 'reopened: the saved answer stays locked');
  });

  testWidgets('Back, Continue later and Next share one footer row in both orientations', (tester) async {
    for (final size in const [Size(1080, 2280), Size(2280, 1080)]) {
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 3;
      addTearDown(tester.view.reset);
      await pumpExam(tester, held: FakeHeldExam());
      await tester.pumpAndSettle();
      final back = tester.getRect(find.text('Back'));
      final later = tester.getRect(find.text('Continue later'));
      final next = tester.getRect(find.text('Next'));
      expect(
        (later.center.dy - next.center.dy).abs() < 1 && (back.center.dy - next.center.dy).abs() < 1,
        isTrue,
        reason: '$size: one row (back $back, later $later, next $next)',
      );
      expect(
        back.right < later.left && later.right < next.left,
        isTrue,
        reason: '$size: Back, Continue later, Next in that order without overlap',
      );
      expect(tester.takeException(), isNull, reason: '$size: no overflow');
    }
  });

  testWidgets('every way out of an answering exam saves the answer currently being edited', (tester) async {
    for (final leave in ['Continue later', 'close button', 'system back']) {
      final log = <String>[];
      final held = FakeHeldExam(log: log);
      await openExam(tester, held: held, client: FakeServerClient(callLog: log));
      await tester.enterText(_field, 'ownership prevents aliasing writes');

      switch (leave) {
        case 'Continue later':
          await tester.tap(find.text('Continue later'));
        case 'close button':
          await tester.tap(find.byIcon(Icons.close));
        default:
          await tester.binding.handlePopRoute();
      }
      await tester.pumpAndSettle();

      expect(find.byType(ExamScreen), findsNothing, reason: '$leave: the screen is left');
      expect(
        held.saved.map(_plain),
        [
          [['ownership prevents aliasing writes', ''], [false, false]],
        ],
        reason: '$leave: the answer visible when the learner pauses is saved as an editable draft',
      );
      expect(log.where((call) => call.startsWith('grading')), isEmpty, reason: '$leave: nothing is sent');
      await tester.pumpWidget(const SizedBox.shrink());
    }
  });

  testWidgets('backgrounding the app saves the typed answer as a draft that resumes editable', (tester) async {
    final held = FakeHeldExam();
    await pumpExam(tester, held: held);
    await tester.enterText(_field, 'half a sent');

    for (final state in const [
      AppLifecycleState.inactive,
      AppLifecycleState.hidden,
      AppLifecycleState.paused,
      AppLifecycleState.hidden,
      AppLifecycleState.inactive,
      AppLifecycleState.resumed,
    ]) {
      tester.binding.handleAppLifecycleStateChanged(state);
    }
    await tester.pump();

    expect(held.saved.map(_plain), [
      [['half a sent', ''], [false, false]],
    ], reason: 'step 1: suspend saves the current text unlocked');

    await tester.pumpWidget(const SizedBox.shrink());
    await pumpExam(tester, held: held);

    expect(find.text('Question 1 of 2'), findsOneWidget, reason: 'step 2: resume opens the draft question');
    final field = tester.widget<TextField>(_field);
    expect(field.controller?.text, 'half a sent', reason: 'step 3: the draft text is back');
    expect(field.readOnly, isFalse, reason: 'step 4: a draft stays editable');
  });

  testWidgets('Android Recents saves the draft before a task-manager kill can skip paused', (tester) async {
    addTearDown(() {
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    });
    final held = FakeHeldExam();
    await pumpExam(tester, held: held);
    await tester.enterText(_field, 'draft visible before opening Recents');

    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    await tester.pump();
    await tester.pumpWidget(const SizedBox.shrink());

    expect(held.saved.map(_plain), [
      [['draft visible before opening Recents', ''], [false, false]],
    ], reason: 'Android reports Recents as inactive, and a task-manager kill may skip paused');
  });

  testWidgets('a system overlay saves without locking and a later edit replaces the snapshot', (tester) async {
    addTearDown(() {
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    });
    final held = FakeHeldExam();
    await pumpExam(tester, held: held);
    await tester.enterText(_field, 'before system dialog');

    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    await tester.pump();
    var field = tester.widget<TextField>(_field);
    expect(field.controller?.text, 'before system dialog', reason: 'step 1: inactive keeps the text');
    expect(field.readOnly, isFalse, reason: 'step 2: inactive does not lock the draft');

    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.enterText(_field, 'after system dialog');
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    await tester.pump();

    expect(held.saved.map(_plain), [
      [['before system dialog', ''], [false, false]],
      [['after system dialog', ''], [false, false]],
    ], reason: 'step 3: each pause saves the text then on screen');
    field = tester.widget<TextField>(_field);
    expect(field.controller?.text, 'after system dialog', reason: 'step 4: later text stays visible');
    expect(field.readOnly, isFalse, reason: 'step 5: the later snapshot stays editable');
  });

  testWidgets('an exam mounted while inactive resumes with its loaded draft still editable', (tester) async {
    addTearDown(() {
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    });
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    final held = FakeHeldExam(answers: ['loaded while inactive', ''], locked: [false, false]);
    await pumpExam(tester, held: held);

    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.pump();

    final field = tester.widget<TextField>(_field);
    expect(field.controller?.text, 'loaded while inactive', reason: 'the draft is loaded');
    expect(field.readOnly, isFalse, reason: 'the initial foreground assumption cannot lock a draft');
    expect(held.saved, isEmpty, reason: 'resuming a screen mounted while inactive does not rewrite it');
  });

  test('examDeckName resolves a workspace member to <workspace>/<file>', () {
    expect(examDeckName(deckPath: '/r/wsfolder/decks/member.md', rootDir: '/r'), 'wsfolder/member.md',
        reason: 'a workspace member drops its decks folder');
    expect(examDeckName(deckPath: '/r/rust.md', rootDir: '/r'), 'rust.md', reason: 'a top-level deck is its file name');
  });
}

Map<String, dynamic> _sentence(String text) => {
  'kind': 'sentence',
  'text': text,
  'runs': [
    {'text': text},
  ],
};

Map<String, dynamic> _code(List<String> lines) => {'kind': 'code', 'lines': lines};
