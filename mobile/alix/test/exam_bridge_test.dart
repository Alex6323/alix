import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bridge/exam_bridge.dart';
import 'package:alix_mobile/src/rust/api/exam.dart' as bridge;
import 'package:alix_mobile/src/rust/frb_generated.dart';

const _deckText = '---\nformat-version: 1\nid: deck-rust\n---\n## q\na\n<!-- id: card-q -->\n';

String _contract(String name) => File('../../tests/contracts/$name.json').readAsStringSync();

({Directory root, File deck, BridgeHeldExam held}) _receivedExam(String prefix) {
  final root = Directory.systemTemp.createTempSync(prefix);
  addTearDown(() => root.deleteSync(recursive: true));
  final deck = File('${root.path}/rust.md')..writeAsStringSync(_deckText);
  final start = bridge.examSittingStart(
    deckPath: deck.path,
    rootDir: root.path,
    needsWorkOnly: false,
    nowMs: BigInt.from(1000),
  );
  final held = BridgeHeldExam(
    deckPath: deck.path,
    deckId: start.deckId,
    rootDir: root.path,
    clock: () => 2000,
  );
  expect(held.view(), isNull, reason: 'before the receive: no file, view is null');
  bridge.heldExamReceive(rootDir: root.path, carriedJson: _contract('CarriedReplyDto'), start: start);
  return (root: root, deck: deck, held: held);
}

void main() {
  setUpAll(RustLib.init);

  test('exam ideas load nothing for a deck without an ideas document', () {
    final root = Directory.systemTemp.createTempSync('alix-exam-ideas-');
    addTearDown(() => root.deleteSync(recursive: true));
    final deck = File('${root.path}/rust.md')..writeAsStringSync(_deckText);

    expect(
      examIdeasLoad(deckPath: deck.path, rootDir: root.path),
      isNull,
      reason: 'a fresh root holds no ideas document, so the load is null',
    );
  });

  test('a held exam is answered, graded, applied and removed through the real bridge', () {
    final (:root, deck: _, :held) = _receivedExam('alix-held-exam-');
    final carried = jsonDecode(_contract('CarriedReplyDto'))['exam'] as Map<String, dynamic>;
    final sitting = carried['sitting'] as String;

    final received = held.view()!;
    expect(
      [received.sitting, received.deck, received.questions, received.answers, received.locked],
      [sitting, carried['deck'], ['Why does Rust use ownership?'], [''], [false]],
      reason: 'received: the view carries the document sitting, deck and question with empty answers',
    );
    expect(
      [received.result, received.outcome, received.canRemediate],
      [null, null, false],
      reason: 'received: no result, no outcome, nothing to remediate',
    );

    held.saveAnswers(const ['ownership frees memory'], const [true]);
    final saved = held.view()!;
    expect(
      [saved.answers, saved.locked],
      [['ownership frees memory'], [true]],
      reason: 'saved: a reload gives back the answer and its lock',
    );

    final tooLong = 'x' * (256 * 1024 + 1);
    expect(
      () => held.saveAnswers([tooLong], const [true]),
      throwsA(isA<AnswerTooLong>()),
      reason: 'lock over the limit: the bridge refusal is raised as AnswerTooLong',
    );
    expect(
      held.view()!.answers,
      ['ownership frees memory'],
      reason: 'lock over the limit: the stored answers stay as they were',
    );
    expect(
      () => held.saveAnswers(const [], const []),
      throwsA(isNot(isA<AnswerTooLong>())),
      reason: 'wrong answer count: another refusal passes through, not as AnswerTooLong',
    );

    held.saveAnswers([tooLong], const [false]);
    expect(
      () => held.gradeRequest(),
      throwsA(isA<AnswerTooLong>()),
      reason: 'an unlocked draft over the limit saves, and the grade request is refused as AnswerTooLong',
    );
    held.saveAnswers(const ['ownership frees memory'], const [true]);

    final request = jsonDecode(held.gradeRequest()) as Map<String, dynamic>;
    expect(
      [request['sitting'], request['answers']],
      [sitting, ['ownership frees memory']],
      reason: 'grade request: JSON naming the sitting and the saved answer',
    );

    expect(
      held.materialChanged(),
      isTrue,
      reason: 'material check: the deck has no sources manifest, so the material is never unchanged and the real bridge reports a change',
    );

    held.storeResult(_contract('GradingReply.results'));
    final stored = held.view()!;
    final grade = stored.result!.grades.single;
    expect(
      [grade.answer, grade.verdict, grade.feedback, stored.outcome],
      ['ownership frees memory', 'FAIL', 'Rust has no GC', null],
      reason: 'stored: the result joins the saved answer with the reply verdict and feedback, before any outcome',
    );
    expect(
      [stored.result!.verdicts.total, stored.result!.topics.single.label, stored.result!.mastered],
      [1, 'Ownership', false],
      reason: 'stored: the counts, the topic label and mastered come from the reply joined with the exam',
    );
    expect(
      () => held.gradeRequest(),
      throwsA(isNot(isA<AnswerTooLong>())),
      reason: 'graded: the grade request refusal is not AnswerTooLong',
    );

    final applied = held.apply();
    expect(
      [applied.outcome?.recorded, applied.outcome?.standing, applied.canRemediate],
      [false, null, false],
      reason: 'applied on a deck the exam does not fit: an outcome that is not recorded, no standing, nothing to remediate',
    );
    expect(
      held.remediationRequest(),
      isNull,
      reason: 'not recorded: no remediation request',
    );

    held.remove();
    expect(held.view(), isNull, reason: 'removed: view is null again');
    expect(root.existsSync(), isTrue, reason: 'removed: only the held file goes');
  });

  test('a result stored on the corpus exam records nothing on a deck it does not fit, and a second apply shows it again', () {
    final (root: _, deck: _, :held) = _receivedExam('alix-held-exam-recorded-');
    held.saveAnswers(const ['ownership frees memory'], const [true]);

    held.storeResult(_contract('GradingReply.results'));
    final applied = held.apply();

    expect(
      [applied.result?.grades.single.verdict, applied.outcome?.recorded, applied.outcome?.standing, applied.canRemediate],
      ['FAIL', false, null, false],
      reason: 'a deck whose material differs from the exam: the real bridge stores "not recorded"',
    );
    expect(
      held.apply().outcome?.recorded,
      false,
      reason: 'a second apply shows the written outcome again',
    );
  });

  test('a view with a recorded outcome and a standing parses its standing and remediated topics', () {
    final view = HeldExamView.fromJson(_viewJson(
      outcome: {
        'recorded': true,
        'standing': {'text': 'passed (4 Nov)', 'exam_changed': true},
      },
      remediated: {'topic-a': 2},
      canRemediate: true,
    ));

    expect(
      [view.outcome?.recorded, view.outcome?.standing?.text, view.outcome?.standing?.examChanged],
      [true, 'passed (4 Nov)', true],
      reason: 'outcome: recorded with its standing text and the changed flag',
    );
    expect(
      [view.remediated, view.canRemediate, view.strictness, view.full, view.unsourcedTopics],
      [{'topic-a': 2}, true, 'balanced', true, ['Terms']],
      reason: 'the remaining fields read as written',
    );
  });

  test('a view document without one of its required fields, or with a wrong type, is refused', () {
    for (final (key, value) in const [
      ('sitting', null),
      ('sitting', 7),
      ('deck', null),
      ('questions', null),
      ('questions', 'Why?'),
      ('answers', null),
      ('locked', null),
      ('locked', ['yes']),
      ('can_remediate', null),
      ('can_remediate', 'true'),
    ]) {
      final json = _viewJson();
      if (value == null) {
        json.remove(key);
      } else {
        json[key] = value;
      }
      expect(
        () => HeldExamView.fromJson(json),
        throwsFormatException,
        reason: '$key set to $value: fromJson refuses the document',
      );
    }
    expect(
      HeldExamView.fromJson(_viewJson()).sitting,
      'sitting-1',
      reason: 'the complete document parses',
    );
  });
}

Map<String, dynamic> _viewJson({
  Map<String, dynamic>? outcome,
  Map<String, dynamic> remediated = const {},
  bool canRemediate = false,
}) => {
  'sitting': 'sitting-1',
  'deck': 'rust.md',
  'strictness': 'balanced',
  'full': true,
  'questions': ['Why?'],
  'questions_units': [<Object>[]],
  'answers': [''],
  'locked': [false],
  'unsourced_topics': ['Terms'],
  'outcome': outcome,
  'remediated': remediated,
  'can_remediate': canRemediate,
  'result': null,
};
