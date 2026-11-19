import 'dart:convert';

import 'package:alix_mobile/bridge/exam_bridge.dart';

/// A [HeldExamAccess] over an in-memory view document. Every call appends
/// its name to [log], which a test may share with the fake client's call
/// log to read one ordered history.
class FakeHeldExam implements HeldExamAccess {
  FakeHeldExam({
    List<String>? log,
    List<String> questions = const ['Why ownership?', 'What is borrowing?'],
    List<String>? answers,
    List<bool>? locked,
    this.sitting = 'sitting-1',
    this.unsourcedTopics = const [],
    this.changed = false,
    this.gradedResult,
    this.standing,
    this.remediable = false,
    this.remediationCount = 1,
    this.applyFailures = 0,
    this.questionsUnits,
  }) : log = log ?? [],
       questions = List.of(questions),
       answers = answers ?? List.filled(questions.length, ''),
       locked = locked ?? List.filled(questions.length, false);

  final List<String> log;
  final String sitting;
  final List<String> questions;
  final List<String> unsourcedTopics;
  final List<Object>? questionsUnits;

  /// The joined result a stored reply shows: `grades`, `topics`, `verdicts`,
  /// `mastered`; each grade's `answer` is filled from the stored answers.
  Map<String, dynamic>? gradedResult;
  ({String text, bool examChanged})? standing;
  bool remediable;
  int remediationCount;
  int applyFailures;

  bool changed;
  Object? viewError;
  bool gone = false;
  Object? materialError;
  Object? storeError;
  Object? remediationError;
  bool Function(List<String> answers, List<bool> locked)? tooLong;
  bool gradeRequestTooLong = false;

  List<String> answers;
  List<bool> locked;
  final List<(List<String>, List<bool>)> saved = [];
  final List<(String, bool)> stored = [];
  final List<String> storedRemediations = [];
  bool? recorded;
  bool? outcomeRecorded;

  bool get hasResult => recorded != null;

  Map<String, dynamic> _document() => {
    'sitting': sitting,
    'deck': 'rust.md',
    'strictness': 'balanced',
    'full': true,
    'questions': questions,
    'questions_units': questionsUnits ?? [for (final _ in questions) <Object>[]],
    'answers': answers,
    'locked': locked,
    'unsourced_topics': unsourcedTopics,
    'result': hasResult ? _joined() : null,
    'outcome': outcomeRecorded == null
        ? null
        : {
            'recorded': outcomeRecorded,
            'standing': outcomeRecorded! && standing != null
                ? {'text': standing!.text, 'exam_changed': standing!.examChanged}
                : null,
          },
    'remediated': <String, int>{},
    'can_remediate': outcomeRecorded == true && remediable,
  };

  Map<String, dynamic> _joined() {
    final result = jsonDecode(jsonEncode(gradedResult ?? _defaultResult())) as Map<String, dynamic>;
    final grades = result['grades'] as List;
    for (var i = 0; i < grades.length && i < answers.length; i++) {
      (grades[i] as Map<String, dynamic>)['answer'] = answers[i];
    }
    return result;
  }

  Map<String, dynamic> _defaultResult() => {
    'grades': [
      for (final question in questions)
        {
          'question': question,
          'answer': '',
          'verdict': 'FAIL',
          'feedback': 'Not quite.',
          'points': ['the point'],
          'missed': ['the point'],
        },
    ],
    'topics': [
      {
        'topic_id': 'topic-a',
        'fingerprint': 'fp-a',
        'label': 'Ownership',
        'cards': ['card-a'],
        'pass': false,
        'verdicts': {'pass': 0, 'partial': 0, 'total': questions.length},
      },
    ],
    'verdicts': {'pass': 0, 'partial': 0, 'total': questions.length},
    'mastered': false,
  };

  HeldExamView _view() => HeldExamView.fromJson(_document());

  @override
  HeldExamView? view() {
    log.add('view');
    if (viewError case final error?) throw error;
    return gone ? null : _view();
  }

  @override
  void saveAnswers(List<String> answers, List<bool> locked) {
    log.add('saveAnswers');
    if (tooLong?.call(answers, locked) ?? false) throw const AnswerTooLong();
    if (hasResult) throw StateError('this exam was already graded');
    this.answers = List.of(answers);
    this.locked = List.of(locked);
    saved.add((List.of(answers), List.of(locked)));
  }

  @override
  bool materialChanged() {
    log.add('materialChanged');
    if (materialError case final error?) throw error;
    return changed;
  }

  @override
  String gradeRequest() {
    log.add('gradeRequest');
    if (gradeRequestTooLong) throw const AnswerTooLong();
    return jsonEncode({'sitting': sitting, 'answers': answers});
  }

  @override
  void storeResult(String replyBody, {required bool recorded}) {
    log.add('storeResult');
    if (storeError case final error?) throw error;
    stored.add((replyBody, recorded));
    this.recorded = recorded;
  }

  @override
  HeldExamView apply() {
    log.add('apply');
    if (applyFailures > 0) {
      applyFailures--;
      throw StateError('cannot write the progress folder');
    }
    outcomeRecorded ??= recorded! && !changed;
    return _view();
  }

  @override
  String? remediationRequest() {
    log.add('remediationRequest');
    return _view().canRemediate ? jsonEncode({'sitting': sitting, 'topics': [0]}) : null;
  }

  @override
  int storeRemediation(String replyBody) {
    log.add('storeRemediation');
    if (remediationError case final error?) throw error;
    storedRemediations.add(replyBody);
    remediable = false;
    return remediationCount;
  }

  @override
  void remove() {
    log.add('remove');
    gone = true;
  }
}
