import 'dart:convert';
import 'dart:io';

import 'package:alix_mobile/bridge/exam_bridge.dart' as exam_bridge;

import 'deck_fixture.dart';

String contract(String name) =>
    File('../../tests/contracts/$name.json').readAsStringSync();

String matchedExamFor(String deck) {
  final carried = contract('CarriedReplyDto');
  final topic =
      ((jsonDecode(carried) as Map)['exam'] as Map)['topics'][0] as Map;
  return carried
      .replaceAll(
        topic['topic_id'] as String,
        exam_bridge.examDeckId(deckPath: deck)!,
      )
      .replaceAll(
        topic['fingerprint'] as String,
        verifiedTestSourceFingerprint,
      );
}
