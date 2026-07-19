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
      final deck = '${root.path}/d.md';
      File(deck)
          .writeAsStringSync('# D\n\n## capital of testland?\nTestville\n');
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
      // Locks the literal null depth passed on tap (not just the resulting
      // FLIP, which Recall's fallback default would also produce if the tap
      // path wrongly coerced to Depth.recall).
      expect(tester.widget<ReviewScreen>(find.byType(ReviewScreen)).depth, isNull);
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
      File('${root.path}/wsSvg/m.md').writeAsStringSync('## q\na\n');

      Directory('${root.path}/wsPng').createSync();
      File('${root.path}/wsPng/alix.toml').writeAsStringSync('title = "WsPng"\n');
      Directory('${root.path}/wsPng/assets').createSync();
      File('${root.path}/wsPng/assets/icon.png').writeAsBytesSync(_pngIcon);
      File('${root.path}/wsPng/m.md').writeAsStringSync('## q\na\n');

      Directory('${root.path}/wsNone').createSync();
      File('${root.path}/wsNone/alix.toml')
          .writeAsStringSync('title = "WsNone"\n');
      File('${root.path}/wsNone/m.md').writeAsStringSync('## q\na\n');

      File('${root.path}/loose.md').writeAsStringSync('# Loose\n\n## q\na\n');
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
      final deck = '${root.path}/base.md';
      File(deck).writeAsStringSync(
          '---\nsource: https://example.com\n---\n# Base\n\n## q?\na\n');
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
      // The picker's listing is read-only and never stamps, so the card
      // needs an explicit id to count as due.
      File('${root.path}/plain.md')
          .writeAsStringSync('# Plain\n\n## q <!-- id: q1 -->\na\n');

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

  group('item 13: mastered window', () {
    /// Two active decks plus one already mastered. There is no frb setter
    /// for mastered progress (it is normally earned by passing the AI
    /// exam); the store is a plain, unversioned JSON file
    /// (`src/store.rs` `StoreFile`), so the fixture writes it directly,
    /// keyed by the deck's subject (its file name) -- `DeckSummary.mastered`
    /// (src/listing.rs `deck_summary`) reads `Store::deck_mastered` alone,
    /// with no dependency on the deck's actual review state.
    Directory mixedRoot() {
      final root = tempRoot('alix-picker-mastered-');
      File('${root.path}/a-active.md')
          .writeAsStringSync('# Active A\n\n## q\na\n');
      File('${root.path}/b-active.md')
          .writeAsStringSync('# Active B\n\n## q\na\n');
      File('${root.path}/z-mastered.md')
          .writeAsStringSync('# Mastered Z\n\n## q\na\n');
      File('${root.path}/progress.json').writeAsStringSync(
        '{"cards": {}, "decks": {"z-mastered.md": {"mastered_at_ms": 1}}}',
      );
      return root;
    }

    testWidgets(
        'the active list tucks a mastered deck behind a Mastered affordance, '
        'and it stays openable from the mastered view', (tester) async {
      final root = mixedRoot();
      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: PickerScreen(root: root.path),
      ));
      await tester.pumpAndSettle();

      expect(find.text('Active A'), findsOneWidget);
      expect(find.text('Active B'), findsOneWidget);
      expect(find.text('Mastered Z'), findsNothing);
      expect(find.text('Mastered · 1'), findsOneWidget);

      await tester.tap(find.text('Mastered · 1'));
      await tester.pumpAndSettle();

      // The mastered view: only the mastered deck, under its own eyebrow.
      expect(find.text('Mastered Z'), findsOneWidget);
      expect(find.text('Active A'), findsNothing);
      expect(find.text('MASTERED 🎉'), findsOneWidget);

      // Still openable, to re-review or cram.
      await tester.tap(find.text('Mastered Z'));
      await tester.pumpAndSettle();
      expect(find.byType(ReviewScreen), findsOneWidget);
    });

    testWidgets('no mastered decks means no affordance', (tester) async {
      final root = tempRoot('alix-picker-no-mastered-');
      File('${root.path}/a.md').writeAsStringSync('# A\n\n## q\na\n');

      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: PickerScreen(root: root.path),
      ));
      await tester.pumpAndSettle();

      expect(find.text('A'), findsOneWidget);
      expect(find.textContaining('Mastered'), findsNothing);
    });
  });

  group('item 14: workspace dependency tree', () {
    /// A requires-chain nested two deep (base -> mid -> tip) plus a loose
    /// sibling, mirroring `list_members_orders_and_indents_a_requires_chain_
    /// like_the_dependency_forest` (src/listing.rs). `base` is sourced (has
    /// an exam) so `mid`/`tip` stay locked behind it until it is mastered --
    /// a source-less chain never gates (`deck::is_locked`), so this is the
    /// smallest fixture that also exercises the locked-dim rendering.
    Directory requiresChainRoot() {
      final root = tempRoot('alix-picker-tree-');
      final ws = Directory('${root.path}/ws')..createSync();
      File('${ws.path}/alix.toml').writeAsStringSync('');
      File('${ws.path}/base.md').writeAsStringSync(
          '---\nsource: https://example.com\n---\n# Base\n\n## q?\na\n');
      File('${ws.path}/mid.md')
          .writeAsStringSync('---\nrequires: base\n---\n# Mid\n\n## q?\na\n');
      File('${ws.path}/tip.md')
          .writeAsStringSync('---\nrequires: mid\n---\n# Tip\n\n## q?\na\n');
      File('${ws.path}/other.md').writeAsStringSync('# Other\n\n## q?\na\n');
      return root;
    }

    testWidgets(
        'member rows render forest order with tree prefixes, and a locked '
        'member is dimmed but still tappable', (tester) async {
      final root = requiresChainRoot();
      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: PickerScreen(root: root.path, dir: '${root.path}/ws'),
      ));
      await tester.pumpAndSettle();

      expect(find.text('Base'), findsOneWidget);
      expect(find.text('Mid'), findsOneWidget);
      expect(find.text('Tip'), findsOneWidget);
      expect(find.text('Other'), findsOneWidget);

      // Mid's own branch line, and Tip's one level deeper (its ancestor's
      // three-space pad plus its own connector): the lean listing's
      // `entry.tree`, drawn as connected guides per `dependency_forest`.
      final guides = tester
          .widgetList<TreeGuides>(find.byType(TreeGuides))
          .map((g) => g.tree)
          .toList();
      expect(guides, containsAll(['└─ ', '   └─ ']));

      // Base and Other are roots: no tree guides in their row.
      final baseRow =
          find.ancestor(of: find.text('Base'), matching: find.byType(InkWell));
      final otherRow = find.ancestor(
          of: find.text('Other'), matching: find.byType(InkWell));
      expect(find.descendant(of: baseRow, matching: find.byType(TreeGuides)),
          findsNothing);
      expect(find.descendant(of: otherRow, matching: find.byType(TreeGuides)),
          findsNothing);

      // Mid and Tip are locked behind the unmastered sourced Base: dimmed,
      // Base itself is not.
      expect(
          tester
              .widget<Opacity>(
                  find.ancestor(of: find.text('Mid'), matching: find.byType(Opacity)))
              .opacity,
          0.5);
      expect(
          tester
              .widget<Opacity>(
                  find.ancestor(of: find.text('Tip'), matching: find.byType(Opacity)))
              .opacity,
          0.5);
      expect(
          tester
              .widget<Opacity>(
                  find.ancestor(of: find.text('Base'), matching: find.byType(Opacity)))
              .opacity,
          1.0);

      // A locked member is still tappable: browse is allowed, the core
      // enforces the lock at session start, not the picker.
      await tester.tap(find.text('Mid'));
      await tester.pumpAndSettle();
      expect(find.byType(ReviewScreen), findsOneWidget);
    });

    testWidgets('the root list (not drilled) shows no tree prefixes',
        (tester) async {
      final root = requiresChainRoot();
      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: PickerScreen(root: root.path),
      ));
      await tester.pumpAndSettle();

      expect(find.text('ws'), findsOneWidget);
      expect(find.byType(TreeGuides), findsNothing);
      expect(find.text('   └─ '), findsNothing);
    });

    testWidgets(
        'a long member title at a deep indent truncates instead of wrapping',
        (tester) async {
      final root = tempRoot('alix-picker-tree-truncate-');
      final ws = Directory('${root.path}/ws')..createSync();
      File('${ws.path}/alix.toml').writeAsStringSync('');
      File('${ws.path}/base.md').writeAsStringSync('# Base\n\n## q?\na\n');
      File('${ws.path}/mid.md')
          .writeAsStringSync('---\nrequires: base\n---\n# Mid\n\n## q?\na\n');
      const longTitle = 'A very long member deck title that would wrap onto '
          'more than one line if the row were not truncating it with an '
          'ellipsis instead';
      File('${ws.path}/deep.md').writeAsStringSync(
          '---\nrequires: mid\n---\n# $longTitle\n\n## q?\na\n');

      await tester.pumpWidget(MaterialApp(
        theme: alixDark(),
        home: PickerScreen(root: root.path, dir: ws.path),
      ));
      await tester.pumpAndSettle();

      // No RenderFlex overflow at the deepest indent (two levels).
      expect(tester.takeException(), isNull);

      final titleWidget = tester.widget<Text>(find.text(longTitle));
      expect(titleWidget.maxLines, 1);
      expect(titleWidget.overflow, TextOverflow.ellipsis);

      final row = find.ancestor(
          of: find.text(longTitle), matching: find.byType(InkWell));
      // The row's fixed minHeight (54, see `_deckRow`); unaffected by indent.
      expect(tester.getSize(row).height, lessThan(70));
    });
  });
}
