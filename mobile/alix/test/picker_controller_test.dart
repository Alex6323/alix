import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/picker/picker_controller.dart';
import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/picker_port.dart';

void main() {
  test('root loading and named mutations publish one coherent state', () {
    final port = _FakePickerPort(
      rootEntries: [_entry('active')],
      conflicts: const ['progress.sync-conflict.json'],
    );
    final controller = PickerController(port: port, root: '/decks');
    var notifications = 0;
    controller.addListener(() => notifications++);

    expect(controller.entries.single.title, 'active');
    expect(controller.conflicts, ['progress.sync-conflict.json']);
    expect(controller.serverReachable, isFalse);
    expect(controller.conflictsDismissed, isFalse);

    controller.setServerReachable(true);
    controller.dismissConflicts();
    port.rootEntries = [_entry('refreshed')];
    controller.reload();

    expect(controller.serverReachable, isTrue);
    expect(controller.conflictsDismissed, isTrue);
    expect(controller.entries.single.title, 'refreshed');
    expect(notifications, 3);
  });

  test('member loading and deadline writes refresh through the port', () {
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
    expect(port.deadlineWrites, [('/decks/workspace', '2026-08-20')]);
    expect(controller.deadline?.date, '2026-08-20');

    port.deadline = null;
    controller.clearDeadline('/decks/workspace');
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
      var notifications = 0;
      controller.addListener(() => notifications++);

      expect(controller.entries.single.title, 'mastered');
      expect(port.listRootCalls, 0);

      await controller.addTutorial();
      expect(port.tutorialRoots, ['/decks']);
      expect(controller.entries.single.title, 'mastered');
      expect(port.listRootCalls, 0);
      expect(notifications, 1);
    },
  );
}

PickerEntry _entry(String title, {bool mastered = false}) {
  return PickerEntry(
    title: title,
    path: '/decks/$title.md',
    isWorkspace: false,
    due: true,
    canRecognize: true,
    isTrace: false,
    lastDepth: PickerDepth.recall,
    mastered: mastered,
    examDue: false,
    hasExam: false,
    locked: false,
    indent: 0,
    tree: '',
  );
}

class _FakePickerPort implements PickerPort {
  _FakePickerPort({
    this.rootEntries = const [],
    this.memberEntries = const [],
    this.conflicts = const [],
    this.deadline,
  });

  List<PickerEntry> rootEntries;
  List<PickerEntry> memberEntries;
  List<String> conflicts;
  PickerDeadline? deadline;
  int listRootCalls = 0;
  final List<(String, String?)> deadlineWrites = [];
  final List<String> tutorialRoots = [];

  @override
  List<PickerEntry> listRoot(String root) {
    listRootCalls++;
    return rootEntries;
  }

  @override
  List<PickerEntry> listMembers({required String root, required String dir}) {
    return memberEntries;
  }

  @override
  List<String> syncConflicts(String root) => conflicts;

  @override
  PickerDeadline? workspaceDeadline({
    required String root,
    required String dir,
  }) {
    return deadline;
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
