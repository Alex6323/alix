import 'dart:convert';
import 'dart:io';

import 'package:alix_mobile/bridge/exam_bridge.dart' as exam_bridge;
import 'package:flutter/foundation.dart';

/// An exam sitting this phone started on a paired desktop: the start state
/// its result is recorded against, and what of that result was applied.
class StartedSitting {
  StartedSitting({
    required this.deckPath,
    required this.start,
    this.sitting,
    this.recorded = false,
    this.standing,
    Map<String, int>? remediated,
  }) : remediated = remediated ?? {};

  final String deckPath;
  final exam_bridge.ExamSittingStart start;
  String? sitting;
  bool recorded;
  ({String text, bool examChanged})? standing;
  final Map<String, int> remediated;

  Map<String, dynamic> toJson() => {
    'deck_path': deckPath,
    'sitting': sitting,
    'cards': start.cards,
    'eligible': start.eligible,
    'sourced': start.sourced,
    'unsourced_cards': start.unsourcedCards,
    'unsourced_topics': start.unsourcedTopics,
    'recorded': recorded,
    'standing': switch (standing) {
      null => null,
      final s => {'text': s.text, 'exam_changed': s.examChanged},
    },
    'remediated': remediated,
  };

  static StartedSitting fromJson(Map<String, dynamic> json) {
    List<String> strings(Object? value) => [for (final s in value as List) s as String];
    final standing = json['standing'] as Map<String, dynamic>?;
    return StartedSitting(
      deckPath: json['deck_path'] as String,
      sitting: json['sitting'] as String?,
      start: exam_bridge.ExamSittingStart(
        cards: json['cards'] == null ? null : strings(json['cards']),
        eligible: json['eligible'] as bool,
        sourced: json['sourced'] as String,
        unsourcedCards: strings(json['unsourced_cards']),
        unsourcedTopics: strings(json['unsourced_topics']),
      ),
      recorded: json['recorded'] as bool,
      standing: standing == null
          ? null
          : (
              text: standing['text'] as String,
              examChanged: standing['exam_changed'] as bool,
            ),
      remediated: {
        for (final entry in (json['remediated'] as Map<String, dynamic>).entries)
          entry.key: entry.value as int,
      },
    );
  }
}

/// The sittings this phone started, one per deck, kept in the app's support
/// folder so an app restart mid-exam still records the result.
class StartedSittings {
  StartedSittings._(this._file, this._byDeck);

  static const _version = 1;
  static final Map<String, StartedSittings> _loaded = {};

  final File _file;
  final Map<String, StartedSitting> _byDeck;

  /// An unreadable file holds no sitting: its results become unrecordable,
  /// as they were before sittings were kept on disk.
  static StartedSittings load(Directory support) {
    final file = File('${support.path}/exam-sittings.json');
    return _loaded.putIfAbsent(file.path, () {
      final byDeck = <String, StartedSitting>{};
      try {
        final json = jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
        if (json['version'] == _version) {
          for (final item in json['sittings'] as List) {
            final sitting = StartedSitting.fromJson(item as Map<String, dynamic>);
            byDeck[sitting.deckPath] = sitting;
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

  /// The kept start for [deckPath], only while it names [sitting].
  StartedSitting? kept(String deckPath, String? sitting) {
    final kept = _byDeck[deckPath];
    if (kept == null || sitting == null || kept.sitting != sitting) return null;
    return kept;
  }

  void put(StartedSitting sitting) {
    _byDeck[sitting.deckPath] = sitting;
    save();
  }

  void remove(String deckPath) {
    if (_byDeck.remove(deckPath) != null) save();
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
