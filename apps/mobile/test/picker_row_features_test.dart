// T5.1a widget tests: the picker row's per-row interaction/visual changes
// (depth remembering, workspace icons, the exam-due marker). Driven against
// the REAL embedded core, mirroring bridge_test.dart's own fixtures/pattern
// (RustLib.init in setUpAll; real deck files on disk; backdating instead of
// sleeping for anything that needs the acquire cooldown behind it).
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

/// The real alix wordmark emblem (assets/alix.svg), reused verbatim so the
/// fixture is a known-good SVG rather than a hand-invented one.
const _svgIcon = '''
<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512">
  <g transform="translate(256 256) scale(1.4) translate(-340 -157.5)" fill="none" stroke="#c2410c" stroke-width="20" stroke-linecap="round" stroke-linejoin="round">
    <path d="M 246 138 C 261.5 138 274 151.4 274 168 C 274 184.6 261.5 198 246 198 C 230.5 198 218 184.6 218 168 C 218 151.4 230.5 138 246 138 Z  M 276 138 L 276 202"/><path d="M 322 113 L 322 202"/><path d="M 368 138 L 368 202"/><path d="M 414 138 L 462 202 M 462 138 L 414 202"/>
    <circle cx="368" cy="112" r="9" fill="#c2410c" stroke="none"/>
  </g>
</svg>
''';

/// A minimal, valid 1x1 PNG (the smallest well-formed PNG byte sequence), so
/// a raster workspace icon decodes cleanly instead of throwing mid-test.
final _pngIcon = base64Decode(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk'
  '+A8AAQUBAScY42YAAAAASUVORK5CYII=',
);

void main() {
  setUpAll(() async => RustLib.init());

  Directory tempRoot(String prefix) {
    final dir = Directory.systemTemp.createTempSync(prefix);
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  group('item 10: depth remembering', () {
    /// A deck acquired and backdated ten minutes (past the 5-min acquire
    /// cooldown), so a real "now" open serves it as a genuine due review
    /// rather than the acquire flow, where tap vs. long-press cannot show
    /// through (a new card looks the same at any depth).
    Directory dueDeckRoot() {
      final root = tempRoot('alix-picker-depth-');
      final deck = '${root.path}/d.txt';
      File(deck)
          .writeAsStringSync('% title: D\n# capital of testland?\n\tTestville\n');
      final backdated =
          BigInt.from(DateTime.now().millisecondsSinceEpoch - 600000);
      ReviewSession.open(deckPath: deck, rootDir: root.path, nowMs: backdated)
          .acquire(nowMs: backdated);
      return root;
    }

    testWidgets('tap opens directly at the remembered depth, no sheet',
        (tester) async {
      final root = dueDeckRoot();
      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: PickerScreen(root: root.path),
      ));
      await tester.pumpAndSettle();

      await tester.tap(find.text('D'));
      await tester.pumpAndSettle();

      // No depth sheet ever showed: the review screen opened straight into
      // the card, at the default (Recall) depth's flip mode.
      expect(find.text('Recognize'), findsNothing);
      expect(find.text('Reconstruct'), findsNothing);
      expect(find.byType(ReviewScreen), findsOneWidget);
      expect(find.text('FLIP'), findsOneWidget);
    });

    testWidgets(
        'long-press shows the sheet with the remembered depth highlighted, and opens with the pick',
        (tester) async {
      final root = dueDeckRoot();
      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: PickerScreen(root: root.path),
      ));
      await tester.pumpAndSettle();

      await tester.longPress(find.text('D'));
      await tester.pumpAndSettle();

      expect(find.text('Recall'), findsOneWidget);
      expect(find.text('Recognize'), findsOneWidget);
      expect(find.text('Reconstruct'), findsOneWidget);

      // Never reviewed before, so the remembered depth is the fallback
      // default (Recall): the check mark sits on that tile only.
      final recallTile = find.ancestor(
          of: find.text('Recall'), matching: find.byType(ListTile));
      final recognizeTile = find.ancestor(
          of: find.text('Recognize'), matching: find.byType(ListTile));
      expect(
          find.descendant(of: recallTile, matching: find.byIcon(Icons.check)),
          findsOneWidget);
      expect(
          find.descendant(
              of: recognizeTile, matching: find.byIcon(Icons.check)),
          findsNothing);

      await tester.tap(find.text('Reconstruct'));
      await tester.pumpAndSettle();

      // Reconstruct + a single-line answer is a typed check, not a flip:
      // proof the picked depth (not the remembered default) is what opened.
      expect(find.byType(ReviewScreen), findsOneWidget);
      expect(find.text('TYPING'), findsOneWidget);
    });
  });

  group('item 11: workspace icons', () {
    Directory iconRoot() {
      final root = tempRoot('alix-picker-icons-');
      Directory('${root.path}/wsSvg').createSync();
      File('${root.path}/wsSvg/alix.toml').writeAsStringSync('title = "WsSvg"\n');
      Directory('${root.path}/wsSvg/assets').createSync();
      File('${root.path}/wsSvg/assets/icon.svg').writeAsStringSync(_svgIcon);
      File('${root.path}/wsSvg/m.txt').writeAsStringSync('# q\n\ta\n');

      Directory('${root.path}/wsPng').createSync();
      File('${root.path}/wsPng/alix.toml').writeAsStringSync('title = "WsPng"\n');
      Directory('${root.path}/wsPng/assets').createSync();
      File('${root.path}/wsPng/assets/icon.png').writeAsBytesSync(_pngIcon);
      File('${root.path}/wsPng/m.txt').writeAsStringSync('# q\n\ta\n');

      Directory('${root.path}/wsNone').createSync();
      File('${root.path}/wsNone/alix.toml')
          .writeAsStringSync('title = "WsNone"\n');
      File('${root.path}/wsNone/m.txt').writeAsStringSync('# q\n\ta\n');

      File('${root.path}/loose.txt')
          .writeAsStringSync('% title: Loose\n# q\n\ta\n');
      return root;
    }

    Finder rowOf(String title) =>
        find.ancestor(of: find.text(title), matching: find.byType(InkWell));

    testWidgets('an svg icon renders a tinted SvgPicture', (tester) async {
      final root = iconRoot();
      await tester
          .pumpWidget(MaterialApp(home: PickerScreen(root: root.path)));
      await tester.pumpAndSettle();

      expect(
          find.descendant(
              of: rowOf('WsSvg'), matching: find.byType(SvgPicture)),
          findsOneWidget);
    });

    testWidgets('a raster icon renders an Image.file', (tester) async {
      final root = iconRoot();
      await tester
          .pumpWidget(MaterialApp(home: PickerScreen(root: root.path)));
      await tester.pumpAndSettle();

      expect(
          find.descendant(of: rowOf('WsPng'), matching: find.byType(Image)),
          findsOneWidget);
    });

    testWidgets('a workspace with no icon renders none', (tester) async {
      final root = iconRoot();
      await tester
          .pumpWidget(MaterialApp(home: PickerScreen(root: root.path)));
      await tester.pumpAndSettle();

      final row = rowOf('WsNone');
      expect(find.descendant(of: row, matching: find.byType(SvgPicture)),
          findsNothing);
      expect(
          find.descendant(of: row, matching: find.byType(Image)), findsNothing);
    });

    testWidgets('a deck row never renders an icon', (tester) async {
      final root = iconRoot();
      await tester
          .pumpWidget(MaterialApp(home: PickerScreen(root: root.path)));
      await tester.pumpAndSettle();

      final row = rowOf('Loose');
      expect(find.descendant(of: row, matching: find.byType(SvgPicture)),
          findsNothing);
      expect(
          find.descendant(of: row, matching: find.byType(Image)), findsNothing);
    });
  });

  group('item 12: exam-due badge', () {
    /// Graduates a single-card `% source:` deck to real FSRS Review state
    /// (never a hand-constructed `FsrsState`): two full Pass grades, each
    /// re-opened past the previous due date, graduate New -> Learning ->
    /// Review (`fsrs_two_goods_graduate_to_review`, src/scheduler.rs). The
    /// FSRS learning step is a hardcoded 10 minutes (rs-fsrs, no test
    /// knob), so the timestamps are anchored to real "now" (PickerScreen's
    /// own listing always reads real wall-clock time, unlike the direct
    /// bridge calls here, which take an explicit nowMs) rather than a fixed
    /// epoch, so the graduated card reads not-due (its next interval lands
    /// days out) instead of overdue-since-1970.
    Directory examDueRoot() {
      final root = tempRoot('alix-picker-examdue-');
      final deck = '${root.path}/base.txt';
      File(deck).writeAsStringSync(
          '% title: Base\n% source: https://example.com\n# q?\n\ta\n');
      final t0 = DateTime.now().millisecondsSinceEpoch - 902000;
      ReviewSession.open(deckPath: deck, rootDir: root.path, nowMs: BigInt.from(t0))
          .acquire(nowMs: BigInt.from(t0));
      ReviewSession.open(
              deckPath: deck, rootDir: root.path, nowMs: BigInt.from(t0 + 301000))
          .grade(grade: Grade.pass, nowMs: BigInt.from(t0 + 301000));
      ReviewSession.open(
              deckPath: deck, rootDir: root.path, nowMs: BigInt.from(t0 + 902000))
          .grade(grade: Grade.pass, nowMs: BigInt.from(t0 + 902000));
      return root;
    }

    testWidgets('an exam-due deck shows the exam marker, not the due dot',
        (tester) async {
      final root = examDueRoot();
      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: PickerScreen(root: root.path),
      ));
      await tester.pumpAndSettle();

      final row =
          find.ancestor(of: find.text('Base'), matching: find.byType(InkWell));
      expect(find.descendant(of: row, matching: find.text('exam')),
          findsOneWidget);
      expect(find.descendant(of: row, matching: find.byIcon(Icons.circle)),
          findsNothing);
    });

    testWidgets('a plain due deck shows the due dot, not the exam marker',
        (tester) async {
      final root = tempRoot('alix-picker-due-');
      File('${root.path}/plain.txt')
          .writeAsStringSync('% title: Plain\n# q\n\ta\n');

      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: PickerScreen(root: root.path),
      ));
      await tester.pumpAndSettle();

      final row = find.ancestor(
          of: find.text('Plain'), matching: find.byType(InkWell));
      expect(find.descendant(of: row, matching: find.byIcon(Icons.circle)),
          findsOneWidget);
      expect(find.descendant(of: row, matching: find.text('exam')),
          findsNothing);
    });
  });
}
