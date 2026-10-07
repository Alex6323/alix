import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/walk_screen.dart';

import 'support/count_rule.dart';
import 'support/deck_fixture.dart';

const _oneSection =
    '# Capitals\nWhere the government sits.\n\n'
    '## capital of italy?\nRome\n\n'
    '## capital of spain?\nMadrid\n';

const _twoSections =
    '# Capitals\nWhere the government sits.\n\n'
    '## capital of italy?\nRome\n\n'
    '## capital of spain?\nMadrid\n\n'
    '# Rivers\nWater that flows.\n\n'
    '## longest river?\nNile\n';

const _longTitle =
    'A deck title long enough that one phone app bar line cannot hold it';

const _titled = '---\ntitle: $_longTitle\n---\n## capital of italy?\nRome\n';

const _choice =
    '## capital of france?\n- [x] Paris\n- [ ] London\n- [ ] Berlin\n'
    '<!-- choices: single -->\n';

void main() {
  setUpAll(() async => RustLib.init());

  final sheetTitle = find.byKey(const ValueKey('section-sheet-title'));

  Future<void> pumpWalk(WidgetTester tester, String deck) async {
    final root = Directory.systemTemp.createTempSync('alix-walk-screen-');
    final support = Directory.systemTemp.createTempSync('alix-walk-support-');
    addTearDown(() {
      root.deleteSync(recursive: true);
      support.deleteSync(recursive: true);
    });
    writeTestDeck('${root.path}/walk.md', deck);
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: WalkScreen(
          key: ValueKey(root.path),
          deckPath: '${root.path}/walk.md',
          rootDir: root.path,
          supportDir: support,
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  Finder chip(String label) => find.widgetWithText(InkWell, label);

  Future<void> tap(WidgetTester tester, String label) async {
    expect(chip(label), findsOneWidget, reason: 'the $label chip is offered');
    await tester.ensureVisible(chip(label));
    await tester.tap(chip(label));
    await tester.pumpAndSettle();
  }

  Future<bool> dismissSheetIfOpen(WidgetTester tester) async {
    if (sheetTitle.evaluate().isEmpty) return false;
    Navigator.of(tester.element(sheetTitle)).pop();
    await tester.pumpAndSettle();
    return true;
  }

  Future<List<bool>> walkOnce(WidgetTester tester) async {
    final opened = <bool>[];
    while (chip('Next walk').evaluate().isEmpty) {
      opened.add(await dismissSheetIfOpen(tester));
      final step = 'item ${opened.length}';
      expect(
        chip('Next'),
        findsNothing,
        reason: '$step: no Next before the attempt',
      );
      await tap(tester, 'Reveal');
      expect(chip('Reveal'), findsNothing, reason: '$step: the answer is open');
      await tap(tester, 'Next');
    }
    return opened;
  }

  testWidgets('law: a section sheet opens on first contact in every walk, '
      'including the next walk of a one-section deck', (tester) async {
    await pumpWalk(tester, _oneSection);
    expect(find.text('2 left'), findsOneWidget, reason: 'open: two items left');
    expect(await walkOnce(tester), [true, false], reason: 'walk 1');
    expect(find.text('walked'), findsOneWidget, reason: 'done: tally row');
    expect(find.text('End of the deck.'), findsOneWidget, reason: 'done');
    await tap(tester, 'Next walk');
    expect(await walkOnce(tester), [true, false], reason: 'walk 2');

    await pumpWalk(tester, _twoSections);
    expect(await walkOnce(tester), [
      true,
      false,
      true,
    ], reason: 'two sections: each opens on its first item');
  });

  testWidgets('a choice item is answered by a pick, then Next', (tester) async {
    await pumpWalk(tester, _choice);
    expect(
      (chip('Reveal').evaluate().length, chip('Next').evaluate().length),
      (0, 0),
      reason: 'front: a pick is the only way in',
    );
    await tester.tap(find.text('Paris'));
    await tester.pumpAndSettle();
    await tap(tester, 'Next');
    expect(find.text('WALK COMPLETE'), findsOneWidget, reason: 'done');
    expect(find.text('1'), findsOneWidget, reason: 'done: one item walked');
  });

  testWidgets('law: the bar names the deck on one truncated line and no '
      'step shows a mode tag', (tester) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);
    await pumpWalk(tester, _titled);
    final title = find.byKey(const ValueKey('walk-title'));
    for (final step in ['front', 'answer', 'done']) {
      expect(
        tester.widget<Text>(title).data,
        _longTitle,
        reason: '$step: the bar carries the deck title',
      );
      final paragraph = tester.renderObject<RenderParagraph>(
        find.descendant(of: title, matching: find.byType(RichText)),
      );
      expect(
        (paragraph.maxLines, paragraph.didExceedMaxLines),
        (1, true),
        reason: '$step: the long title is cut to one line, not wrapped',
      );
      expectCountRule(tester, title, step);
      if (step == 'front') await tap(tester, 'Reveal');
      if (step == 'answer') await tap(tester, 'Next');
    }
  });
}
