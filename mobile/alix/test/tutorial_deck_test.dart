// The picker's empty-state "Add the tutorial deck" action: a folder that
// never got the first-run seed (a shared folder, or an emptied one) can still
// start the tutorial. The copy itself (addTutorialDeck) is a plain async unit
// test against the real bundle; the button's presence/absence is a widget
// test (its listing calls the embedded core, so RustLib.init() is required).
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';

void main() {
  setUpAll(() async => RustLib.init());

  Directory temp(String prefix) {
    final dir = Directory.systemTemp.createTempSync(prefix);
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  test('addTutorialDeck copies the bundled tutorial into an empty folder', () async {
    final root = temp('alix-tut-copy-');
    final file = File('${root.path}/tutorial.md');
    expect(file.existsSync(), isFalse);

    await addTutorialDeck(root.path);

    expect(file.existsSync(), isTrue);
    expect(file.readAsStringSync().trim(), isNotEmpty);
  });

  test('addTutorialDeck leaves an existing tutorial.md untouched', () async {
    final root = temp('alix-tut-skip-');
    final file = File('${root.path}/tutorial.md');
    file.writeAsStringSync('## mine\nkeep\n');

    await addTutorialDeck(root.path);

    expect(
      file.readAsStringSync(),
      '## mine\nkeep\n',
      reason: 'never overwrites an existing tutorial deck',
    );
  });

  testWidgets('an empty root offers to add the tutorial deck', (tester) async {
    final support = temp('alix-tut-support-');
    final root = temp('alix-tut-decks-');
    await tester.pumpWidget(
      MaterialApp(
        home: PickerScreen(
          root: root.path,
          onSetDecksDir: (_) async {},
          supportDir: support,
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Add the tutorial deck'), findsOneWidget);
  });

  testWidgets('a drilled-in empty workspace does not offer the tutorial', (
    tester,
  ) async {
    final support = temp('alix-tut-support-');
    final root = temp('alix-tut-decks-');
    // A drill-in level passes `dir`; the tutorial belongs at the decks root,
    // never inside a workspace, so the button must not appear here.
    await tester.pumpWidget(
      MaterialApp(
        home: PickerScreen(
          root: root.path,
          dir: root.path,
          title: 'Some workspace',
          onSetDecksDir: (_) async {},
          supportDir: support,
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Add the tutorial deck'), findsNothing);
  });
}
