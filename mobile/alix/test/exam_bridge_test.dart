import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bridge/exam_bridge.dart' as exam_bridge;
import 'package:alix_mobile/src/rust/frb_generated.dart';

void main() {
  setUpAll(RustLib.init);

  test('exam ideas round trip through the phone store', () {
    final root = Directory.systemTemp.createTempSync('alix-exam-ideas-');
    addTearDown(() => root.deleteSync(recursive: true));
    final deck = File('${root.path}/rust.md')
      ..writeAsStringSync(
        '---\nformat-version: 1\nid: deck-rust\n---\n## q\na\n<!-- id: card-q -->\n',
      );

    expect(
      exam_bridge.examIdeasLoad(deckPath: deck.path, rootDir: root.path),
      isNull,
    );

    const first = '{"version":1,"material":"xxh64-first","topics":[]}';
    exam_bridge.examIdeasSave(
      deckPath: deck.path,
      rootDir: root.path,
      json: first,
    );
    expect(
      jsonDecode(exam_bridge.examIdeasLoad(deckPath: deck.path, rootDir: root.path)!),
      jsonDecode(first),
    );

    const second = '{"version":1,"material":"xxh64-second","topics":[]}';
    exam_bridge.examIdeasSave(
      deckPath: deck.path,
      rootDir: root.path,
      json: second,
    );
    expect(
      jsonDecode(exam_bridge.examIdeasLoad(deckPath: deck.path, rootDir: root.path)!),
      jsonDecode(second),
    );
    expect(
      jsonDecode(File('${root.path}/.alix/exam/deck-rust.json').readAsStringSync()),
      jsonDecode(second),
    );
    expect(
      () => exam_bridge.examIdeasSave(
        deckPath: deck.path,
        rootDir: root.path,
        json: '{"version":1,"unknown":true}',
      ),
      throwsA(anything),
    );
  });

  test('exam answers resume only the exact question list', () {
    final root = Directory.systemTemp.createTempSync('alix-exam-answers-');
    addTearDown(() => root.deleteSync(recursive: true));
    final deck = File('${root.path}/rust.md')
      ..writeAsStringSync(
        '---\nformat-version: 1\nid: deck-rust\n---\n## q\na\n<!-- id: card-q -->\n',
      );
    const questions = ['Why ownership?', 'What is borrowing?'];

    expect(
      exam_bridge.examAnswersLoad(
        deckPath: deck.path,
        rootDir: root.path,
        questions: questions,
      ),
      ['', ''],
    );
    exam_bridge.examAnswersSave(
      deckPath: deck.path,
      rootDir: root.path,
      questions: questions,
      answers: const ['ownership controls destruction', ''],
    );
    expect(
      exam_bridge.examAnswersLoad(
        deckPath: deck.path,
        rootDir: root.path,
        questions: questions,
      ),
      ['ownership controls destruction', ''],
    );

    final prepared = File('${root.path}/.alix/exam/deck-rust.prepared.json');
    expect(prepared.existsSync(), isTrue);
    expect(
      exam_bridge.examAnswersLoad(
        deckPath: deck.path,
        rootDir: root.path,
        questions: const ['Replacement?'],
      ),
      [''],
    );
    expect(prepared.existsSync(), isFalse);

    exam_bridge.examAnswersSave(
      deckPath: deck.path,
      rootDir: root.path,
      questions: const ['Replacement?'],
      answers: const ['answer'],
    );
    exam_bridge.examAnswersClear(deckPath: deck.path, rootDir: root.path);
    expect(prepared.existsSync(), isFalse);
  });
}
