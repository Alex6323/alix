import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/picker/picker_controller.dart';
import 'package:alix_mobile/profile.dart';
import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/picker_port.dart';

void main() {
  test(
    'with ALIX_PROFILE the controller prints one alix-profile line per listing',
    () async {
      const profile = PickerProfile(
        libMs: 7,
        counters: [('decks_loaded', 3), ('manifest_reads', 1)],
      );
      final lines = <String>[];
      final previous = debugPrint;
      debugPrint = (String? message, {int? wrapWidth}) {
        lines.add(message ?? '');
      };
      addTearDown(() => debugPrint = previous);

      PickerController(
        port: _FakePickerPort(rootEntries: [_entry('a')], profile: profile),
        root: '/decks',
      );
      PickerController(
        port: _FakePickerPort(memberEntries: [_entry('m')], profile: profile),
        root: '/decks',
        dir: '/decks/ws',
      );
      await pumpEventQueue();

      expect(lines, [
        matches(
          RegExp(
            r'^alix-profile root bridge_ms=\d+ lib_ms=7 decks_loaded=3 manifest_reads=1$',
          ),
        ),
        matches(
          RegExp(
            r'^alix-profile members bridge_ms=\d+ lib_ms=7 decks_loaded=3 manifest_reads=1$',
          ),
        ),
      ]);
    },
    skip: kAlixProfile ? false : 'needs --dart-define=ALIX_PROFILE=true',
  );

  test('the list is loading until the first listing answers', () async {
    final port = _FakePickerPort(rootEntries: [_entry('active')]);
    final controller = PickerController(port: port, root: '/decks');
    expect(controller.isLoading, isTrue, reason: 'nothing has answered yet');
    expect(controller.entries, isEmpty, reason: 'no rows before the answer');

    await pumpEventQueue();

    expect(controller.isLoading, isFalse, reason: 'the first listing answered');
    expect(controller.entries.single.title, 'active');
  });

  test('root loading and named mutations publish one coherent state', () async {
    final port = _FakePickerPort(rootEntries: [_entry('active')]);
    final controller = PickerController(port: port, root: '/decks');
    await pumpEventQueue();
    var notifications = 0;
    controller.addListener(() => notifications++);

    expect(controller.entries.single.title, 'active');
    expect(controller.serverReachable, isFalse);

    controller.setServerReachable(true);
    port.rootEntries = [_entry('refreshed')];
    controller.reload();
    await pumpEventQueue();

    expect(controller.serverReachable, isTrue);
    expect(controller.entries.single.title, 'refreshed');
    expect(notifications, 2);
  });

  test('member loading and deadline writes refresh through the port', () async {
    final port = _FakePickerPort(
      memberEntries: [_entry('member')],
      deadline: const PickerDeadline(
        date: '2026-08-10',
        daysLeft: 9,
        ready: 1,
        total: 3,
      ),
    );
    final controller = PickerController(
      port: port,
      root: '/decks',
      dir: '/decks/workspace',
    );
    await pumpEventQueue();
    var notifications = 0;
    controller.addListener(() => notifications++);

    expect(controller.entries.single.title, 'member');
    expect(controller.deadline?.date, '2026-08-10');

    port.deadline = const PickerDeadline(
      date: '2026-08-20',
      daysLeft: 19,
      ready: 1,
      total: 3,
    );
    controller.setDeadline(dir: '/decks/workspace', date: '2026-08-20');
    await pumpEventQueue();
    expect(port.deadlineWrites, [('/decks/workspace', '2026-08-20')]);
    expect(controller.deadline?.date, '2026-08-20');

    port.deadline = null;
    controller.clearDeadline('/decks/workspace');
    await pumpEventQueue();
    expect(port.deadlineWrites.last, ('/decks/workspace', null));
    expect(controller.deadline, isNull);
    expect(notifications, 2);
  });

  test(
    'mastered entries bypass listing and tutorial reload is named',
    () async {
      final port = _FakePickerPort(rootEntries: [_entry('from bridge')]);
      final controller = PickerController(
        port: port,
        root: '/decks',
        masteredEntries: [_entry('mastered', mastered: true)],
      );
      await pumpEventQueue();
      var notifications = 0;
      controller.addListener(() => notifications++);

      expect(controller.entries.single.title, 'mastered');
      expect(port.listRootCalls, 0);

      await controller.addTutorial();
      await pumpEventQueue();
      expect(port.tutorialRoots, ['/decks']);
      expect(controller.entries.single.title, 'mastered');
      expect(port.listRootCalls, 0);
      expect(notifications, 1);
    },
  );
  test(
    'rows publish before their strips, which fill in when the core answers',
    () async {
      final port = _FakePickerPort(
        rootEntries: [_entry('deck'), _entry('ws', isWorkspace: true)],
      )..stripReply = Completer();
      final controller = PickerController(port: port, root: '/decks');
      await pumpEventQueue();
      var notifications = 0;
      controller.addListener(() => notifications++);

      expect(
        controller.entries.map((e) => e.title),
        ['deck', 'ws'],
        reason: 'the listing publishes without waiting on the strips',
      );
      expect(controller.isLoading, isFalse, reason: 'listed before strips');
      expect(controller.stripFor('/decks/deck.md'), isNull);
      expect(
        [for (final (root, decks) in port.stripRequests) '$root: $decks'],
        ['/decks: [/decks/deck.md]'],
        reason: 'one request per listing, never for a workspace row',
      );

      port.stripReply!.complete({
        '/decks/deck.md': const PickerStrip(
          cardCount: 2,
          tiers: ['seen', 'unseen'],
        ),
      });
      await pumpEventQueue();

      expect(controller.stripFor('/decks/deck.md')?.cardCount, 2);
      expect(notifications, 1, reason: 'the strip answer repaints once');
    },
  );

  test(
    'an older strip answer cannot overwrite the latest relisting',
    () async {
      final first = Completer<Map<String, PickerStrip>>();
      final second = Completer<Map<String, PickerStrip>>();
      final port = _FakePickerPort(rootEntries: [_entry('deck')])
        ..stripReplies.addAll([first, second]);
      final controller = PickerController(port: port, root: '/decks');
      await pumpEventQueue();

      controller.reload();
      await pumpEventQueue();
      expect(port.stripRequests, hasLength(2));

      second.complete({
        '/decks/deck.md': const PickerStrip(
          cardCount: 2,
          tiers: ['seen', 'unseen'],
        ),
      });
      await pumpEventQueue();
      expect(controller.stripFor('/decks/deck.md')?.cardCount, 2);

      first.complete({
        '/decks/deck.md': const PickerStrip(
          cardCount: 1,
          tiers: ['seen'],
        ),
      });
      await pumpEventQueue();

      expect(
        controller.stripFor('/decks/deck.md')?.cardCount,
        2,
        reason: 'a late answer for the old listing must be ignored',
      );
    },
  );

  test(
    'search lists every root once and keeps exactly the matching rows',
    () async {
      final port = _FakePickerPort(rootEntries: [_entry('own')]);
      final controller = PickerController(port: port, root: '/decks');
      controller.setPairedRoot('/paired');
      await pumpEventQueue();
      const labels = ['Rust Basics', 'rusty Nails', 'Go', 'TRUST me', 'Ruby'];
      port.searchable = [
        for (final (index, label) in labels.indexed)
          PickerSearchHit(
            root: index.isEven ? '/decks' : '/paired',
            entry: _entry(label),
          ),
      ];

      controller.openSearch();
      await pumpEventQueue();
      expect(port.searchRoots, [
        ['/decks', '/paired'],
      ]);

      for (final query in ['rust', ' RUST ', 'u', 'zzz', 'Go']) {
        controller.setQuery(query);
        final needle = query.trim().toLowerCase();
        expect(
          controller.searchHits!.map((hit) => hit.entry.title),
          [
            for (final label in labels)
              if (label.toLowerCase().contains(needle)) label,
          ],
          reason: 'query "$query"',
        );
      }
      controller.closeSearch();
      expect(controller.isSearching, isFalse);
    },
  );

  test(
    'a relisting queues a search refresh while the old search is in flight',
    () async {
      final first = Completer<List<PickerSearchHit>>();
      final second = Completer<List<PickerSearchHit>>();
      final port = _FakePickerPort(rootEntries: [_entry('own')])
        ..searchReplies.addAll([first, second]);
      final controller = PickerController(port: port, root: '/decks');
      await pumpEventQueue();

      controller.openSearch();
      await pumpEventQueue();
      expect(port.searchRoots, [
        ['/decks'],
      ]);

      port.rootEntries = [_entry('refreshed')];
      controller.reload();
      await pumpEventQueue();
      first.complete([
        PickerSearchHit(root: '/decks', entry: _entry('stale')),
      ]);
      await pumpEventQueue();

      expect(
        port.searchRoots,
        [
          ['/decks'],
          ['/decks'],
        ],
        reason: 'the relisting refresh must not be dropped',
      );
      second.complete([
        PickerSearchHit(root: '/decks', entry: _entry('fresh')),
      ]);
      await pumpEventQueue();
      expect(controller.searchHits!.single.entry.title, 'fresh');
    },
  );

  test(
    'a queued relisting refresh does not publish the stale search first',
    () async {
      final first = Completer<List<PickerSearchHit>>();
      final second = Completer<List<PickerSearchHit>>();
      final port = _FakePickerPort(rootEntries: [_entry('own')])
        ..searchReplies.addAll([first, second]);
      final controller = PickerController(port: port, root: '/decks');
      await pumpEventQueue();

      controller.openSearch();
      await pumpEventQueue();
      controller.reload();
      await pumpEventQueue();

      first.complete([
        PickerSearchHit(root: '/decks', entry: _entry('stale')),
      ]);
      await pumpEventQueue();

      expect(
        controller.searchHits,
        isNull,
        reason: 'the answer invalidated by the relisting must not be published',
      );

      second.complete([
        PickerSearchHit(root: '/decks', entry: _entry('fresh')),
      ]);
      await pumpEventQueue();
      expect(controller.searchHits!.single.entry.title, 'fresh');
    },
  );

  test(
    'a relisting still in flight does not publish the stale search first',
    () async {
      final first = Completer<List<PickerSearchHit>>();
      final second = Completer<List<PickerSearchHit>>();
      final relisting = Completer<PickerListing>();
      final port = _FakePickerPort(rootEntries: [_entry('own')])
        ..searchReplies.addAll([first, second]);
      final controller = PickerController(port: port, root: '/decks');
      await pumpEventQueue();

      controller.openSearch();
      await pumpEventQueue();
      port.rootReply = relisting;
      controller.reload();
      await pumpEventQueue();

      first.complete([
        PickerSearchHit(root: '/decks', entry: _entry('stale')),
      ]);
      await pumpEventQueue();

      expect(
        controller.searchHits,
        isNull,
        reason: 'a relisting invalidates the old search when it starts',
      );

      relisting.complete(
        PickerListing(entries: [_entry('refreshed')]),
      );
      await pumpEventQueue();
      second.complete([
        PickerSearchHit(root: '/decks', entry: _entry('fresh')),
      ]);
      await pumpEventQueue();
      expect(controller.searchHits!.single.entry.title, 'fresh');
    },
  );

  test('a strip answer from the stale search cannot overwrite the refreshed '
      'search strip', () async {
    final firstSearch = Completer<List<PickerSearchHit>>();
    final secondSearch = Completer<List<PickerSearchHit>>();
    final staleStrip = Completer<Map<String, PickerStrip>>();
    final freshStrip = Completer<Map<String, PickerStrip>>();
    final port = _FakePickerPort(rootEntries: [_entry('ws', isWorkspace: true)])
      ..searchReplies.addAll([firstSearch, secondSearch])
      ..stripReplies.addAll([staleStrip, freshStrip]);
    final controller = PickerController(port: port, root: '/decks');
    await pumpEventQueue();

    controller.openSearch();
    await pumpEventQueue();
    firstSearch.complete([
      PickerSearchHit(root: '/decks', entry: _entry('deck')),
    ]);
    await pumpEventQueue();

    controller.closeSearch();
    controller.openSearch();
    await pumpEventQueue();
    secondSearch.complete([
      PickerSearchHit(root: '/decks', entry: _entry('deck')),
    ]);
    await pumpEventQueue();
    expect(port.stripRequests, hasLength(2));

    freshStrip.complete({
      '/decks/deck.md': const PickerStrip(
        cardCount: 2,
        tiers: ['seen', 'unseen'],
      ),
    });
    await pumpEventQueue();
    expect(controller.stripFor('/decks/deck.md')?.cardCount, 2);

    staleStrip.complete({
      '/decks/deck.md': const PickerStrip(cardCount: 1, tiers: ['seen']),
    });
    await pumpEventQueue();

    expect(
      controller.stripFor('/decks/deck.md')?.cardCount,
      2,
      reason: 'a strip answer issued by the stale search must be ignored',
    );
  });

  test('closing or disposing drops a queued search refresh', () async {
    final calls = <String, int>{};
    for (final (label, stop) in <(String, void Function(PickerController))>[
      ('closeSearch', (controller) => controller.closeSearch()),
      ('dispose', (controller) => controller.dispose()),
    ]) {
      final firstSearch = Completer<List<PickerSearchHit>>();
      final port = _FakePickerPort(
        rootEntries: [_entry('ws', isWorkspace: true)],
      )..searchReplies.add(firstSearch);
      final controller = PickerController(port: port, root: '/decks');
      await pumpEventQueue();

      controller.openSearch();
      await pumpEventQueue();
      controller.reload();
      await pumpEventQueue();
      stop(controller);
      firstSearch.complete(const []);
      await pumpEventQueue();
      calls[label] = port.searchRoots.length;
      if (label != 'dispose') controller.dispose();
    }

    expect(calls, {
      'closeSearch': 1,
      'dispose': 1,
    }, reason: 'a stopped search must not launch its queued full rescan');
  });

  test(
    'a failed search reports no uncaught error and can be retried',
    () async {
      final port = _FakePickerPort(
        rootEntries: [_entry('ws', isWorkspace: true)],
      )..searchFailures = 1;
      final errors = <Object>[];
      final exercised = Completer<void>();
      late PickerController controller;
      runZonedGuarded(() async {
        controller = PickerController(port: port, root: '/decks');
        await pumpEventQueue();
        controller.openSearch();
        await pumpEventQueue();
        controller.closeSearch();
        controller.openSearch();
        await pumpEventQueue();
        exercised.complete();
      }, (error, _) => errors.add(error));
      await exercised.future;
      controller.dispose();

      expect(
        (requests: port.searchRoots.length, uncaught: errors.length),
        (requests: 2, uncaught: 0),
        reason: 'one failed optional search must not wedge every later search',
      );
    },
  );

  test(
    'a failed strip request stays contained as optional enrichment',
    () async {
      final port = _FakePickerPort(rootEntries: [_entry('deck')])
        ..stripFailures = 1;
      final errors = <Object>[];
      final exercised = Completer<void>();
      late PickerController controller;
      runZonedGuarded(() async {
        controller = PickerController(port: port, root: '/decks');
        await pumpEventQueue();
        exercised.complete();
      }, (error, _) => errors.add(error));
      await exercised.future;
      controller.dispose();

      expect(
        errors,
        isEmpty,
        reason:
            'an optional strip failure must not escape as an uncaught error',
      );
    },
  );
}

PickerEntry _entry(
  String title, {
  bool mastered = false,
  bool isWorkspace = false,
}) {
  return PickerEntry(
    title: title,
    path: isWorkspace ? '/decks/$title' : '/decks/$title.md',
    isWorkspace: isWorkspace,
    due: true,
    canRecognize: true,
    isTrace: false,
    mastered: mastered,
    examDue: false,
    hasExam: false,
    locked: false,
    progressError: false,
    indent: 0,
    tree: '',
  );
}

class _FakePickerPort implements PickerPort {
  _FakePickerPort({
    this.rootEntries = const [],
    this.memberEntries = const [],
    this.deadline,
    this.profile,
  });

  List<PickerEntry> rootEntries;
  List<PickerEntry> memberEntries;
  PickerDeadline? deadline;
  PickerProfile? profile;
  int listRootCalls = 0;
  Completer<PickerListing>? rootReply;
  final List<(String, String?)> deadlineWrites = [];
  final List<String> tutorialRoots = [];
  final List<(String, List<String>)> stripRequests = [];
  Completer<Map<String, PickerStrip>>? stripReply;
  final List<Completer<Map<String, PickerStrip>>> stripReplies = [];
  List<PickerSearchHit> searchable = const [];
  final List<List<String>> searchRoots = [];
  final List<Completer<List<PickerSearchHit>>> searchReplies = [];
  int searchFailures = 0;
  int stripFailures = 0;

  @override
  Future<Map<String, PickerStrip>> deckStrips({
    required String root,
    required List<String> decks,
  }) {
    stripRequests.add((root, decks));
    if (stripFailures > 0) {
      stripFailures--;
      return Future.error(StateError('strip failed'));
    }
    if (stripReplies.isNotEmpty) return stripReplies.removeAt(0).future;
    return stripReply?.future ?? Future.value(const {});
  }

  @override
  Future<List<PickerSearchHit>> listSearchable(List<String> roots) async {
    searchRoots.add(roots);
    if (searchFailures > 0) {
      searchFailures--;
      throw StateError('search failed');
    }
    if (searchReplies.isNotEmpty) return searchReplies.removeAt(0).future;
    return searchable;
  }

  @override
  Future<PickerListing> listRoot(String root, {required bool profile}) async {
    listRootCalls++;
    final reply = rootReply;
    if (reply != null) return reply.future;
    return PickerListing(entries: rootEntries, profile: this.profile);
  }

  @override
  Future<PickerListing> listMembers({
    required String root,
    required String dir,
    required bool profile,
  }) async {
    return PickerListing(
      entries: memberEntries,
      deadline: deadline,
      profile: this.profile,
    );
  }

  @override
  void setWorkspaceDeadline({required String dir, required String? date}) {
    deadlineWrites.add((dir, date));
  }

  @override
  Future<void> addTutorialDeck(String root) async {
    tutorialRoots.add(root);
  }

  @override
  String get coreVersion => 'test';

  @override
  String applyGeneratedDeck({
    required String decksDir,
    required String filename,
    required String text,
  }) {
    return '$decksDir/$filename';
  }
}
