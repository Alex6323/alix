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

    const first = '{"version":1,"material":"xxh64-first","sections":[]}';
    exam_bridge.examIdeasSave(
      deckPath: deck.path,
      rootDir: root.path,
      json: first,
    );
    expect(
      exam_bridge.examIdeasLoad(deckPath: deck.path, rootDir: root.path),
      first,
    );

    const second = '{"version":1,"material":"xxh64-second","sections":[]}';
    exam_bridge.examIdeasSave(
      deckPath: deck.path,
      rootDir: root.path,
      json: second,
    );
    expect(
      exam_bridge.examIdeasLoad(deckPath: deck.path, rootDir: root.path),
      second,
    );
    expect(File('${root.path}/.alix/exam/deck-rust.json').readAsStringSync(), second);
  });
}
