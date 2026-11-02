import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/review/review_view.dart' show ReviewCantOpenView;
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/settings_screen.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/walk_screen.dart';

import 'support/deck_fixture.dart';
import 'support/fake_server_client.dart';
import 'support/picker_listing.dart';

void main() {
  setUp(answerPathProvider);
  setUpAll(() async => RustLib.init());

  final sheetTitle = find.byKey(const ValueKey('section-sheet-title'));

  Directory temp(String prefix) {
    final dir = Directory.systemTemp.createTempSync(prefix);
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  Future<void> pumpPicker(
    WidgetTester tester, {
    required Directory root,
    required Directory support,
    String? dir,
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: PickerScreen(
          root: root.path,
          dir: dir,
          title: dir == null ? null : 'Some workspace',
          supportDir: support,
          currentThemeId: 'dark',
          onSetTheme: (_) async {},
        ),
      ),
    );
    await settlePicker(tester);
  }

  void expectTutorialWalk(Directory support, String step) {
    expect(find.byType(WalkScreen), findsOneWidget, reason: '$step: a walk');
    expect(find.byType(ReviewCantOpenView), findsNothing,
        reason: '$step: it opened');
    expect(sheetTitle, findsOneWidget,
        reason: '$step: the tutorial section opens on its first card');
    expect(File('${support.path}/tutorial/tutorial.md').existsSync(), isTrue,
        reason:
            '$step: the tutorial lives in the support dir\'s tutorial folder');
  }

  testWidgets('the Settings Tutorial row opens the tutorial walk over the '
      'picker', (tester) async {
    final support = temp('alix-tut-support-');
    final root = temp('alix-tut-decks-');
    writeTestDeck('${root.path}/mine.md', '---\ntitle: Mine\n---\n## q\na\n');
    await pumpPicker(tester, root: root, support: support);

    await tester.tap(find.byIcon(Icons.menu));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(ListTile, 'Tutorial'));
    await tester.pumpAndSettle();

    expectTutorialWalk(support, 'settings row');
    expect(find.byType(SettingsScreen), findsNothing,
        reason: 'settings row: Settings pops before the walk is pushed');
    expect(Directory(root.path).listSync().map((e) => e.path),
        ['${root.path}/mine.md'],
        reason: 'settings row: nothing lands in the decks folder');

    Navigator.of(tester.element(sheetTitle)).pop();
    await tester.pumpAndSettle();
    await tester.pageBack();
    await tester.pumpAndSettle();
    expect(find.byType(WalkScreen), findsNothing,
        reason: 'back: the walk closes');
    expect(find.byType(SettingsScreen), findsNothing,
        reason: 'back: not Settings');
    expect(find.byType(PickerScreen), findsOneWidget,
        reason: 'back: the picker');
  });

  testWidgets('the empty picker\'s Tutorial button opens the tutorial walk',
      (tester) async {
    final support = temp('alix-tut-support-');
    final root = temp('alix-tut-decks-');
    await pumpPicker(tester, root: root, support: support);

    await tester.tap(find.text('Tutorial'));
    await tester.pumpAndSettle();

    expectTutorialWalk(support, 'empty picker button');
    expect(Directory(root.path).listSync(), isEmpty,
        reason: 'empty picker button: nothing lands in the decks folder');
  });

  testWidgets('a support folder that cannot be found shows a snack, not a '
      'crash', (tester) async {
    final root = temp('alix-tut-decks-');
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: PickerScreen(
          root: root.path,
          currentThemeId: 'dark',
          onSetTheme: (_) async {},
        ),
      ),
    );
    await settlePicker(tester);
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
      const MethodChannel('plugins.flutter.io/path_provider'),
      (call) async => throw PlatformException(code: 'unavailable'),
    );

    await tester.tap(find.text('Tutorial'));
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 50)),
    );
    await tester.pumpAndSettle();

    expect(find.text('could not open the tutorial.'), findsOneWidget);
    expect(find.byType(WalkScreen), findsNothing);
  });

  testWidgets('a drilled-in empty workspace does not offer the tutorial',
      (tester) async {
    final support = temp('alix-tut-support-');
    final root = temp('alix-tut-decks-');
    await pumpPicker(tester, root: root, support: support, dir: root.path);

    expect(find.text('Tutorial'), findsNothing);
  });

  testWidgets('tutorial card 7 hides its revealed answer and brings it back',
      (tester) async {
    final support = temp('alix-tut-support-');
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: WalkScreen.tutorial(
          baseDir: '${support.path}/tutorial',
          supportDir: support,
        ),
      ),
    );
    await tester.pumpAndSettle();
    Navigator.of(tester.element(sheetTitle)).pop();
    await tester.pumpAndSettle();

    Finder chip(String label) => find.widgetWithText(InkWell, label);
    Future<void> tapChip(String label) async {
      await tester.tap(chip(label));
      await tester.pumpAndSettle();
    }

    for (var number = 1; number <= 6; number++) {
      if (number == 4) {
        await tester.tap(find.text('Got it'));
        await tester.pumpAndSettle();
        await tapChip('I guessed');
        continue;
      }
      while (chip('Got it').evaluate().isEmpty) {
        final label = chip('Reveal next').evaluate().isEmpty
            ? 'Reveal'
            : 'Reveal next';
        await tapChip(label);
      }
      await tapChip('Got it');
    }

    expect(
      find.text(
        'Reveal this answer, hide it again, then bring it back. '
        'What is hiding for?',
      ),
      findsOneWidget,
      reason: 'the shipped tutorial reached card 7',
    );
    await tapChip('Reveal');
    final answerTap = find.byKey(const ValueKey('answer-tap'));
    await tester.tap(answerTap);
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('hide-sheet')), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('hide-sheet-close')));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('hide-sheet')), findsNothing);
    expect(
      find.text(
        'A second try at recalling before you grade, with the answer out of '
        'sight.',
      ),
      findsOneWidget,
      reason: 'closing the hide sheet brings the card 7 answer back',
    );
  });

  group('the tutor sheet on a walk', () {
    Future<void> openTutor(WidgetTester tester, Widget walk) async {
      await tester.pumpWidget(MaterialApp(theme: alixDark(), home: walk));
      await tester.pumpAndSettle();
      if (sheetTitle.evaluate().isNotEmpty) {
        Navigator.of(tester.element(sheetTitle)).pop();
        await tester.pumpAndSettle();
      }
      final reveal = find.widgetWithText(InkWell, 'Reveal');
      await tester.ensureVisible(reveal);
      await tester.tap(reveal);
      await tester.pumpAndSettle();
      final ask = find.widgetWithText(InkWell, 'Ask');
      expect(ask, findsOneWidget, reason: 'a live paired desktop offers Ask');
      await tester.ensureVisible(ask);
      await tester.tap(ask);
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('tutor-question-field')), findsOneWidget,
          reason: 'the tutor sheet is open');
    }

    Future<Directory> pairedSupport() async {
      final support = temp('alix-tut-support-');
      await savePairing(
        const ServerConfig(
          host: '127.0.0.1',
          port: 7777,
          token: 'tok',
          rootId: 'root-test0000000000000000000000',
        ),
        support: support,
      );
      return support;
    }

    final makeCard = find.byKey(const ValueKey('tutor-make-card-button'));
    final makeNote = find.byKey(const ValueKey('tutor-make-note-button'));

    testWidgets('an ordinary walk offers Make card and Note', (tester) async {
      final support = await pairedSupport();
      final root = temp('alix-tut-decks-');
      writeTestDeck('${root.path}/walk.md', '## capital of italy?\nRome\n');
      await openTutor(
        tester,
        WalkScreen(
          deckPath: '${root.path}/walk.md',
          rootDir: root.path,
          supportDir: support,
          buildClient: (_) => FakeServerClient(versionReply: '0.6.0'),
        ),
      );

      expect(makeCard, findsOneWidget);
      expect(makeNote, findsOneWidget);
    });

    testWidgets('a tutorial walk offers Ask but no Make card or Note',
        (tester) async {
      final support = await pairedSupport();
      await openTutor(
        tester,
        WalkScreen.tutorial(
          baseDir: '${support.path}/tutorial',
          supportDir: support,
          buildClient: (_) => FakeServerClient(versionReply: '0.6.0'),
        ),
      );

      expect(makeCard, findsNothing);
      expect(makeNote, findsNothing);
    });
  });
}
