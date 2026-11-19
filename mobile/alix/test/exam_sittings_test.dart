import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/exam_sittings.dart';

const _recordKeys = [
  'root',
  'deck_id',
  'deck_path',
  'title',
  'sitting',
  'announced',
  'cards',
  'eligible',
  'sourced',
  'unsourced_cards',
  'unsourced_topics',
  'fetched',
];

Map<String, dynamic> _record({
  String root = 'root-a',
  String deckId = 'deck-1',
  String sitting = 'sitting-1',
}) => {
  'root': root,
  'deck_id': deckId,
  'deck_path': 'decks/$deckId.md',
  'title': 'Deck $deckId',
  'sitting': sitting,
  'announced': true,
  'cards': ['card-a', 'card-b'],
  'eligible': true,
  'sourced': 'xxh64-0123456789abcdef',
  'unsourced_cards': ['card-c'],
  'unsourced_topics': ['topic-x'],
  'fetched': false,
};

StartedSitting _sitting({
  String root = 'root-a',
  String deckId = 'deck-1',
  String sitting = 'sitting-1',
  List<String>? cards = const ['card-a', 'card-b'],
}) => StartedSitting(
  root: root,
  deckId: deckId,
  deckPath: 'decks/$deckId.md',
  title: 'Deck $deckId',
  sitting: sitting,
  announced: true,
  fetched: false,
  start: ExamSittingStart(
    deckId: deckId,
    cards: cards,
    eligible: true,
    sourced: 'xxh64-0123456789abcdef',
    unsourcedCards: const ['card-c'],
    unsourcedTopics: const ['topic-x'],
  ),
);

void main() {
  late Directory support;

  setUp(() {
    StartedSittings.forgetLoaded();
    support = Directory.systemTemp.createTempSync('exam-sittings-test');
    addTearDown(() => support.deleteSync(recursive: true));
  });

  File file() => File('${support.path}/exam-sittings.json');

  void writeFile(List<Object?> records, {Object version = 1}) {
    file().writeAsStringSync(jsonEncode({'version': version, 'sittings': records}));
  }

  StartedSittings reload() {
    StartedSittings.forgetLoaded();
    return StartedSittings.load(support);
  }

  test('a start record round-trips with every key', () {
    final kept = StartedSittings.load(support);
    kept.put(_sitting());
    kept.put(_sitting(deckId: 'deck-full', sitting: 'sitting-full', cards: null));

    final written = jsonDecode(file().readAsStringSync()) as Map<String, dynamic>;
    final records = (written['sittings'] as List).cast<Map<String, dynamic>>();
    expect(written['version'], 1, reason: 'file version after put');
    for (final record in records) {
      expect(
        record.keys.toSet(),
        _recordKeys.toSet(),
        reason: 'keys of the record for ${record['deck_id']} after put',
      );
    }

    final back = reload();
    final partial = back.forDeck('root-a', 'deck-1');
    expect(partial, isNotNull, reason: 'forDeck(root-a, deck-1) after reload');
    expect(partial!.root, 'root-a', reason: 'root after reload');
    expect(partial.deckId, 'deck-1', reason: 'deckId after reload');
    expect(partial.deckPath, 'decks/deck-1.md', reason: 'deckPath after reload');
    expect(partial.title, 'Deck deck-1', reason: 'title after reload');
    expect(partial.sitting, 'sitting-1', reason: 'sitting after reload');
    expect(partial.announced, isTrue, reason: 'announced after reload');
    expect(partial.fetched, isFalse, reason: 'fetched after reload');
    expect(partial.start.deckId, 'deck-1', reason: 'start.deckId after reload');
    expect(partial.start.cards, ['card-a', 'card-b'], reason: 'start.cards after reload');
    expect(partial.start.eligible, isTrue, reason: 'start.eligible after reload');
    expect(partial.start.sourced, 'xxh64-0123456789abcdef', reason: 'start.sourced after reload');
    expect(partial.start.unsourcedCards, ['card-c'], reason: 'start.unsourcedCards after reload');
    expect(partial.start.unsourcedTopics, ['topic-x'], reason: 'start.unsourcedTopics after reload');
    expect(
      back.forDeck('root-a', 'deck-full')?.start.cards,
      isNull,
      reason: 'start.cards of a full sitting after reload',
    );
  });

  test('records are kept per root and deck id, and a new start for that deck replaces the old one', () {
    final kept = StartedSittings.load(support);
    kept.put(_sitting());
    kept.put(_sitting(deckId: 'deck-2', sitting: 'sitting-2'));
    kept.put(_sitting(root: 'root-b', sitting: 'sitting-3'));

    var back = reload();
    expect(back.forDeck('root-a', 'deck-1')?.sitting, 'sitting-1', reason: 'root-a deck-1 kept beside the others');
    expect(back.forDeck('root-a', 'deck-2')?.sitting, 'sitting-2', reason: 'a second deck is its own record');
    expect(back.forDeck('root-b', 'deck-1')?.sitting, 'sitting-3', reason: 'the same deck id under root-b is its own record');
    expect(back.forDeck('root-b', 'deck-2'), isNull, reason: 'root-b deck-2 was never started');

    back.put(_sitting(sitting: 'sitting-4'));
    back = reload();
    expect(back.forDeck('root-a', 'deck-1')?.sitting, 'sitting-4', reason: 'a new start replaces root-a deck-1');
    expect(back.bySitting('sitting-1'), isNull, reason: 'the replaced sitting-1 is gone');
    expect(back.bySitting('sitting-4')?.deckId, 'deck-1', reason: 'bySitting(sitting-4)');
    expect(back.bySitting(null), isNull, reason: 'bySitting(null)');
    expect(back.forDeck('root-a', 'deck-2')?.sitting, 'sitting-2', reason: 'root-a deck-2 untouched by the replace');
    expect(back.forDeck('root-b', 'deck-1')?.sitting, 'sitting-3', reason: 'root-b deck-1 untouched by the replace');

    back.remove('root-a', 'deck-1');
    back = reload();
    expect(back.forDeck('root-a', 'deck-1'), isNull, reason: 'root-a deck-1 after remove');
    expect(back.forDeck('root-b', 'deck-1')?.sitting, 'sitting-3', reason: 'root-b deck-1 after removing root-a deck-1');
    expect(back.forDeck('root-a', 'deck-2')?.sitting, 'sitting-2', reason: 'root-a deck-2 after removing deck-1');
  });

  test('a start records file whose records carry a key beyond the record holds no sitting', () {
    writeFile([_record(deckId: 'deck-1'), {..._record(deckId: 'deck-2'), 'invented_extra': 'x'}]);

    final back = StartedSittings.load(support);

    expect(back.forDeck('root-a', 'deck-2'), isNull, reason: 'the record carrying invented_extra');
    expect(back.forDeck('root-a', 'deck-1'), isNull, reason: 'the complete record beside it, the whole file is unreadable');
  });

  test('a record missing one key makes the whole file hold no sitting', () {
    for (final key in _recordKeys) {
      StartedSittings.forgetLoaded();
      writeFile([_record(deckId: 'deck-1'), {..._record(deckId: 'deck-2')}..remove(key)]);

      final back = StartedSittings.load(support);

      expect(back.forDeck('root-a', 'deck-2'), isNull, reason: 'the record without $key');
      expect(back.forDeck('root-a', 'deck-1'), isNull, reason: 'the complete record beside the one without $key');
    }
  });

  test('a record with a value of the wrong type makes the whole file hold no sitting', () {
    for (final key in _recordKeys) {
      StartedSittings.forgetLoaded();
      writeFile([_record(deckId: 'deck-1'), {..._record(deckId: 'deck-2'), key: 7}]);

      final back = StartedSittings.load(support);

      expect(back.forDeck('root-a', 'deck-2'), isNull, reason: 'the record with a number for $key');
      expect(back.forDeck('root-a', 'deck-1'), isNull, reason: 'the complete record beside the one with a bad $key');
    }
  });

  test('a file with another version holds no sitting', () {
    writeFile([_record()], version: 2);

    expect(StartedSittings.load(support).forDeck('root-a', 'deck-1'), isNull, reason: 'forDeck in a version 2 file');
  });

  test('a complete file is read', () {
    writeFile([_record()]);

    expect(StartedSittings.load(support).forDeck('root-a', 'deck-1')?.sitting, 'sitting-1', reason: 'forDeck in a complete file');
  });
}
