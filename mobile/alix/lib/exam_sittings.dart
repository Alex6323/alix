import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

/// A sitting's starting state, evaluated once against the phone's store:
/// the cards to ask about (null for a full sitting), whether the review
/// condition holds, and the evidence stamp a standing records.
class ExamSittingStart {
  const ExamSittingStart({
    required this.deckId,
    required this.cards,
    required this.eligible,
    required this.sourced,
    required this.unsourcedCards,
    required this.unsourcedTopics,
  });

  final String deckId;
  final List<String>? cards;
  final bool eligible;
  final String sourced;
  final List<String> unsourcedCards;
  final List<String> unsourcedTopics;
}

class StartedSitting {
  StartedSitting({
    required this.root,
    required this.deckId,
    required this.deckPath,
    required this.title,
    required this.sitting,
    required this.start,
    this.announced = false,
    this.fetched = false,
  });

  final String root;
  final String deckId;
  final String deckPath;
  final String title;
  final String sitting;
  final ExamSittingStart start;
  bool announced;
  bool fetched;

  static const _keys = {
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
  };

  Map<String, dynamic> toJson() => {
    'root': root,
    'deck_id': deckId,
    'deck_path': deckPath,
    'title': title,
    'sitting': sitting,
    'announced': announced,
    'cards': start.cards,
    'eligible': start.eligible,
    'sourced': start.sourced,
    'unsourced_cards': start.unsourcedCards,
    'unsourced_topics': start.unsourcedTopics,
    'fetched': fetched,
  };

  static StartedSitting fromJson(Map<String, dynamic> json) {
    if (json.length != _keys.length || !json.keys.every(_keys.contains)) {
      throw const FormatException('a start record has exactly its keys');
    }
    List<String> strings(Object? value) => [for (final s in value as List) s as String];
    final deckId = json['deck_id'] as String;
    return StartedSitting(
      root: json['root'] as String,
      deckId: deckId,
      deckPath: json['deck_path'] as String,
      title: json['title'] as String,
      sitting: json['sitting'] as String,
      announced: json['announced'] as bool,
      start: ExamSittingStart(
        deckId: deckId,
        cards: json['cards'] == null ? null : strings(json['cards']),
        eligible: json['eligible'] as bool,
        sourced: json['sourced'] as String,
        unsourcedCards: strings(json['unsourced_cards']),
        unsourcedTopics: strings(json['unsourced_topics']),
      ),
      fetched: json['fetched'] as bool,
    );
  }
}

class StartedSittings {
  StartedSittings._(this._file, this._byDeck);

  static const _version = 1;
  static final Map<String, StartedSittings> _loaded = {};

  final File _file;
  final Map<(String, String), StartedSitting> _byDeck;

  /// An unreadable file holds no sitting.
  static StartedSittings load(Directory support) {
    final file = File('${support.path}/exam-sittings.json');
    return _loaded.putIfAbsent(file.path, () {
      final byDeck = <(String, String), StartedSitting>{};
      try {
        final json = jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
        if (json['version'] == _version) {
          for (final item in json['sittings'] as List) {
            final sitting = StartedSitting.fromJson(item as Map<String, dynamic>);
            byDeck[(sitting.root, sitting.deckId)] = sitting;
          }
        }
      } on Object {
        byDeck.clear();
      }
      return StartedSittings._(file, byDeck);
    });
  }

  /// Drops the in-memory copies, so the next [load] reads the disk: what an
  /// app restart does.
  @visibleForTesting
  static void forgetLoaded() => _loaded.clear();

  StartedSitting? forDeck(String root, String deckId) => _byDeck[(root, deckId)];

  StartedSitting? bySitting(String? sitting) {
    if (sitting == null) return null;
    for (final kept in _byDeck.values) {
      if (kept.sitting == sitting) return kept;
    }
    return null;
  }

  void put(StartedSitting sitting) {
    _byDeck[(sitting.root, sitting.deckId)] = sitting;
    save();
  }

  void remove(String root, String deckId) {
    if (_byDeck.remove((root, deckId)) != null) save();
  }

  void save() {
    final tmp = File('${_file.path}.tmp');
    tmp.writeAsStringSync(
      jsonEncode({
        'version': _version,
        'sittings': [for (final s in _byDeck.values) s.toJson()],
      }),
    );
    tmp.renameSync(_file.path);
  }
}
