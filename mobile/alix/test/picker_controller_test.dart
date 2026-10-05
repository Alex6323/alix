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
  final List<(String, String?)> deadlineWrites = [];
  final List<String> tutorialRoots = [];
  final List<(String, List<String>)> stripRequests = [];
  Completer<Map<String, PickerStrip>>? stripReply;
  List<PickerSearchHit> searchable = const [];
  final List<List<String>> searchRoots = [];

  @override
  Future<Map<String, PickerStrip>> deckStrips({
    required String root,
    required List<String> decks,
  }) {
    stripRequests.add((root, decks));
    return stripReply?.future ?? Future.value(const {});
  }

  @override
  Future<List<PickerSearchHit>> listSearchable(List<String> roots) async {
    searchRoots.add(roots);
    return searchable;
  }

  @override
  Future<PickerListing> listRoot(String root, {required bool profile}) async {
    listRootCalls++;
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
