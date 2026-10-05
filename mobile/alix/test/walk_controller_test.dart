import 'dart:io';
import 'dart:ui';

import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bridge/walk_bridge.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/walk/walk_controller.dart';
import 'package:alix_mobile/walk/walk_models.dart';

import 'support/deck_fixture.dart';

const _deck =
    '## capital of france?\n- [x] Paris\n- [ ] London\n- [ ] Berlin\n'
    '<!-- choices: single -->\n\n'
    '## even numbers\n- [x] 2\n- [x] 4\n- [ ] 3\n'
    '<!-- choices: multiple -->\n\n'
    '## capital of italy?\nRome\n\n'
    '## Draw it\nか\n<!-- input: draw -->\n';

void main() {
  setUpAll(() async => RustLib.init());

  WalkController open() {
    final root = Directory.systemTemp.createTempSync('alix-walk-controller-');
    addTearDown(() => root.deleteSync(recursive: true));
    writeTestDeck('${root.path}/walk.md', _deck);
    final controller = WalkController(
      factory: const WalkBridgeFactory(),
      deckPath: '${root.path}/walk.md',
      rootDir: root.path,
    );
    addTearDown(controller.dispose);
    return controller;
  }

  test('law: every item opens on its front, an attempt opens its answer, and '
      'Next clears the attempt', () {
    final controller = open();
    expect(controller.openError, isNull, reason: 'open: the deck walks');
    final total = controller.state.total;
    expect(total, 4, reason: 'open: four items');
    final kinds = <String>{};
    for (var step = 0; step < total; step++) {
      final state = controller.state;
      final label = 'step $step (${state.card?.front})';
      expect(state.phase, WalkPhase.front, reason: '$label: front');
      expect(state.position, step, reason: '$label: position');
      expect(
        (controller.choice, controller.multiChoice, controller.revealed),
        (null, null, false),
        reason: '$label: no attempt carried over',
      );
      expect(controller.multiSelected, isEmpty, reason: '$label: no picks');
      expect(controller.sketch.isEmpty, isTrue, reason: '$label: blank sketch');

      final options = state.choices;
      if (options != null && state.choicesMultiple == true) {
        kinds.add('multi');
        expect(state.mode, ReviewMode.choice, reason: '$label: picks');
        controller.toggleChoice(options.indexOf('2'));
        controller.toggleChoice(options.indexOf('4'));
        controller.submitChoices();
        expect(
          controller.multiChoice?.passed,
          isTrue,
          reason: '$label: the correct set passes',
        );
      } else if (options != null) {
        kinds.add('single');
        expect(state.mode, ReviewMode.choice, reason: '$label: picks');
        controller.choose(options.indexOf('Paris'));
        expect(
          controller.choice?.passed,
          isTrue,
          reason: '$label: the correct pick passes',
        );
      } else {
        kinds.add(state.input == ReviewInput.draw ? 'draw' : 'flip');
        expect(state.mode, ReviewMode.flip, reason: '$label: flips');
        if (state.input == ReviewInput.draw) {
          controller.sketchBegin(const Offset(1, 1), PointerDeviceKind.touch);
          controller.sketchExtend(const Offset(5, 5));
          controller.sketchEnd();
        }
        controller.reveal();
        expect(controller.revealed, isTrue, reason: '$label: revealed');
        expect(
          controller.sketch.isEmpty,
          state.input != ReviewInput.draw,
          reason: '$label: the reveal keeps the sketch beside the answer',
        );
      }
      expect(
        controller.state.phase,
        WalkPhase.answer,
        reason: '$label: the attempt opens the answer',
      );
      controller.next();
    }
    expect(kinds, {
      'single',
      'multi',
      'flip',
      'draw',
    }, reason: 'the deck covers every item kind');
    expect(controller.state.phase, WalkPhase.done, reason: 'end: done');
    expect(controller.state.card, isNull, reason: 'end: no card');

    var notifications = 0;
    controller.addListener(() => notifications++);
    controller.nextWalk();
    expect(
      (controller.state.phase, controller.state.position, notifications),
      (WalkPhase.front, 0, 1),
      reason: 'next walk: back to the first item, one notification',
    );
  });

  test('an open failure is reported, not thrown', () {
    final root = Directory.systemTemp.createTempSync('alix-walk-missing-');
    addTearDown(() => root.deleteSync(recursive: true));
    final controller = WalkController(
      factory: const WalkBridgeFactory(),
      deckPath: '${root.path}/absent.md',
      rootDir: root.path,
    );
    addTearDown(controller.dispose);
    expect(controller.openError, isNotNull);
  });
}
