import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/bridge/sync_bridge.dart' as sync_bridge;
import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/picker_view.dart';
import 'package:alix_mobile/picker/picker_widgets.dart';
import 'package:alix_mobile/picker/tree_guides.dart';
import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

import 'support/deck_fixture.dart';
import 'support/fake_server_client.dart';
import 'support/fake_sync_port.dart';
import 'support/picker_listing.dart';

void main() {
  setUp(answerPathProvider);
  setUpAll(() async => RustLib.init());

  Directory tempRoot(String prefix) {
    final dir = Directory.systemTemp.createTempSync(prefix);
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  String cards(int count) =>
      [for (var i = 0; i < count; i++) '## q$i\na$i\n'].join('\n');

  void workspace(String dir, String title, Map<String, String> members) {
    Directory('$dir/decks').createSync(recursive: true);
    File('$dir/alix.toml').writeAsStringSync('title = "$title"\n');
    for (final MapEntry(:key, :value) in members.entries) {
      writeTestDeck('$dir/decks/$key.md', value);
    }
  }

  Finder rowOf(String title) => find
      .ancestor(of: find.text(title), matching: find.byType(InkWell))
      .first;

  Finder stripIn(String title) =>
      find.descendant(of: rowOf(title), matching: find.byType(PickerTierStrip));

  bool stripFilled(WidgetTester tester, String title) =>
      find.text(title).evaluate().isNotEmpty &&
      stripIn(title).evaluate().isNotEmpty &&
      tester.widget<PickerTierStrip>(stripIn(title)).strip != null;

  Future<void> pumpPicker(
    WidgetTester tester,
    Widget home, {
    required bool Function() until,
  }) async {
    await tester.pumpWidget(MaterialApp(theme: alixDark(), home: home));
    await settlePicker(tester, until: until);
  }

  testWidgets(
    'deck and member rows carry a strip and a workspace row carries none',
    (tester) async {
      final root = tempRoot('alix-strip-kinds-');
      writeTestDeck('${root.path}/loose.md', '---\ntitle: Loose\n---\n${cards(3)}');
      workspace('${root.path}/ws', 'Ws', {
        'base': '---\ntitle: Base\n---\n${cards(2)}',
        'mid': '---\nrequires: base\ntitle: Mid\n---\n${cards(1)}',
      });

      await pumpPicker(
        tester,
        PickerScreen(root: root.path),
        until: () => stripFilled(tester, 'Loose'),
      );
      expect(stripIn('Loose'), findsOneWidget, reason: 'a root deck row');
      expect(stripIn('Ws'), findsNothing, reason: 'a root workspace row');
      expect(tester.widget<PickerTierStrip>(stripIn('Loose')).strip?.cardCount, 3);

      await pumpPicker(
        tester,
        PickerScreen(
          key: UniqueKey(),
          root: root.path,
          dir: '${root.path}/ws',
          title: 'Ws',
        ),
        until: () => stripFilled(tester, 'Base') && stripFilled(tester, 'Mid'),
      );
      expect(
        find.descendant(of: rowOf('Mid'), matching: find.byType(TreeGuides)),
        findsOneWidget,
        reason: 'Mid is the tree-guided member row form',
      );
      for (final (title, count) in [('Base', 2), ('Mid', 1)]) {
        expect(
          tester.widget<PickerTierStrip>(stripIn(title)).strip?.tiers,
          List.filled(count, 'unseen'),
          reason: '$title: one unseen cell per card',
        );
      }
    },
  );

  testWidgets(
    'a deck row has no placeholder icon and a workspace keeps its initial',
    (tester) async {
      final root = tempRoot('alix-strip-icon-');
      writeTestDeck('${root.path}/loose.md', '---\ntitle: Loose\n---\n${cards(1)}');
      workspace('${root.path}/ws', 'Workspace', {'m': cards(1)});

      await pumpPicker(
        tester,
        PickerScreen(root: root.path),
        until: () => stripFilled(tester, 'Loose'),
      );

      expect(find.byIcon(Icons.style_outlined), findsNothing);
      expect(
        find.descendant(of: rowOf('Workspace'), matching: find.text('W')),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'the card count sits in a fixed slot so every strip ends at the same x',
    (tester) async {
      final root = tempRoot('alix-strip-slot-');
      for (final (name, count) in [('one', 1), ('ten', 12), ('hundred', 120)]) {
        writeTestDeck('${root.path}/$name.md', cards(count));
      }

      await pumpPicker(
        tester,
        PickerScreen(root: root.path),
        until: () => ['one', 'ten', 'hundred'].every(
          (title) => stripFilled(tester, title),
        ),
      );

      final slots = find.byKey(const ValueKey('picker-strip-count'));
      final bars = find.byKey(const ValueKey('picker-strip-bar'));
      expect(slots, findsNWidgets(3));
      expect(find.text('120'), findsOneWidget);
      expect(
        {for (final slot in slots.evaluate()) tester.getRect(find.byWidget(slot.widget))},
        hasLength(3),
        reason: 'three rows, three distinct slots',
      );
      expect(
        {for (final slot in slots.evaluate()) tester.getSize(find.byWidget(slot.widget)).width},
        {PickerTierStrip.countWidth},
        reason: 'the count slot has one width whatever the digit count',
      );
      expect(
        {for (final bar in bars.evaluate()) tester.getRect(find.byWidget(bar.widget)).right},
        hasLength(1),
        reason: 'every strip ends at the same x',
      );
    },
  );

  testWidgets(
    'rows render before their strip data and keep their height when it fills',
    (tester) async {
      PickerEntry entry(String title) => PickerEntry(
        title: title,
        path: '/decks/$title.md',
        isWorkspace: false,
        due: true,
        canRecognize: true,
        isTrace: false,
        mastered: false,
        examDue: false,
        hasExam: false,
        locked: false,
        progressError: false,
        indent: 0,
        tree: '',
      );
      Widget view(PickerStrip? Function(String) stripFor) => MaterialApp(
        theme: alixDark(),
        home: PickerView(
          entries: [entry('Deck')],
          deadline: null,
          isRoot: true,
          isMasteredView: false,
          leading: const SizedBox(width: 56),
          onOpenEntry: (_) {},
          onLongPressEntry: (_) {},
          onOpenMastered: (_) {},
          onAddTutorial: () {},
          stripFor: stripFor,
        ),
      );

      await tester.pumpWidget(view((_) => null));
      expect(find.text('Deck'), findsOneWidget);
      expect(tester.widget<PickerTierStrip>(stripIn('Deck')).strip, isNull);
      expect(find.byKey(const ValueKey('picker-strip-count')), findsOneWidget);
      final before = tester.getSize(rowOf('Deck')).height;

      await tester.pumpWidget(
        view((_) => const PickerStrip(cardCount: 42, tiers: ['seen'])),
      );
      expect(find.text('42'), findsOneWidget);
      expect(tester.getSize(rowOf('Deck')).height, before);
    },
  );

  testWidgets(
    'search shows exactly the rows whose label holds the query across '
    'local, synced, and member rows',
    (tester) async {
      const rootId = 'root-test0000000000000000000000';
      const config = ServerConfig(
        host: '127.0.0.1',
        port: 7777,
        token: 'abc',
        rootId: rootId,
      );
      final support = tempRoot('alix-search-support-');
      await savePairing(config, support: support);
      final paired = Directory(
        sync_bridge.pairedRootDirFor(support: support.path, rootId: rootId),
      )..createSync(recursive: true);
      final phone = tempRoot('alix-search-phone-');
      writeTestDeck('${phone.path}/a.md', '---\ntitle: Alpha Deck\n---\n${cards(1)}');
      workspace('${phone.path}/ws', 'Beta Ws', {
        'm': '---\ntitle: alpha member\n---\n${cards(1)}',
        'n': '---\nrequires: m\ntitle: Kappa chained\n---\n${cards(1)}',
      });
      writeTestDeck('${paired.path}/g.md', '---\ntitle: Gamma ALPHA\n---\n${cards(1)}');
      workspace('${paired.path}/ws', 'Delta', {
        'e': '---\ntitle: Epsilon\n---\n${cards(1)}',
      });
      const labels = [
        'Alpha Deck',
        'Beta Ws',
        'alpha member',
        'Kappa chained',
        'Gamma ALPHA',
        'Delta',
        'Epsilon',
      ];
      final port = FakeSyncPort(rootId: rootId, rootDir: paired.path);

      await pumpPicker(
        tester,
        PickerScreen(
          root: phone.path,
          supportDir: support,
          currentThemeId: 'dark',
          onSetTheme: (_) async {},
          buildClient: (_) => FakeServerClient(versionReply: minServerVersion),
          buildSyncPort: (_, _) => port,
        ),
        until: () => find.text('Gamma ALPHA').evaluate().isNotEmpty,
      );
      await tester.tap(find.byTooltip('Search'));
      await tester.pump();

      for (final query in ['alpha', 'ALPHA ', 'ta', 'e', 'chained']) {
        await tester.enterText(
          find.byKey(const ValueKey('picker-search-field')),
          query,
        );
        await settlePicker(
          tester,
          until: () => find.byType(PickerSearchEmpty).evaluate().isNotEmpty ||
              labels.any((l) => find.text(l).evaluate().isNotEmpty),
        );
        final needle = query.trim().toLowerCase();
        for (final label in labels) {
          expect(
            find.text(label),
            label.toLowerCase().contains(needle) ? findsOneWidget : findsNothing,
            reason: 'query "$query", row "$label"',
          );
        }
        expect(find.byType(TreeGuides), findsNothing, reason: 'results are flat');
        expect(find.byType(PickerSearchEmpty), findsNothing);
      }

      await tester.enterText(
        find.byKey(const ValueKey('picker-search-field')),
        'zzz',
      );
      await tester.pump();
      for (final label in labels) {
        expect(find.text(label), findsNothing, reason: 'query "zzz", "$label"');
      }
      expect(find.text('No decks match.'), findsOneWidget);
    },
  );
}
