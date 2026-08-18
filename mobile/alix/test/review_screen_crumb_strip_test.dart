// The crumb strip (T5.6): the "where am I" region breadcrumb rendered as
// the first child of the review screen's body, under the AppBar, when a
// session is topology-ordered.
//
// ReviewCrumbModel is a plain Dart data class; render tests below pump
// CrumbStrip directly against
// hand-built fixtures - deterministic, fast, no native init needed. That
// sidesteps a real wall: building a genuine topology-ordered session needs
// the deck's actual Card::id hashes, and the mobile bridge exposes no way
// to read a card's id from Dart (CardView carries front/back/etc but no
// id; mintTutorCard's returned id belongs to an unrelated minted virtual,
// not a deck card). CLAUDE.md also forbids hand-computing Card::id outside
// the lib ("a wrong id fails silently"), so re-deriving the hash in the
// test is not an option either. See the task report for the full writeup.
//
// The null-path and "review still renders" checks below DO drive the real
// embedded core (ReviewScreen over a plain fact deck, mirroring
// review_screen_ask_chip_test.dart's pumpReview pattern), since that path
// needs no card id at all: a deck with no cached topology always crumbs
// null.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/crumb_strip.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

import 'support/deck_fixture.dart';

void main() {
  setUpAll(() async => RustLib.init());

  Future<void> pumpStrip(WidgetTester tester, ReviewCrumbModel crumb) async {
    await tester.pumpWidget(MaterialApp(
      theme: alixDark(),
      home: Scaffold(
        appBar: AppBar(title: const Text('t')),
        body: CrumbStrip(crumb: crumb),
      ),
    ));
  }

  testWidgets('a topology crumb renders its region names and banded tier cells',
      (tester) async {
    final crumb = ReviewCrumbModel(
      regions: const ['Intro', 'Body'],
      current: 1,
      cells: const [
        ['learned-weak'],
        ['learned-strong'],
      ],
    );
    await pumpStrip(tester, crumb);

    final tokens = alixDark().alix;
    expect(find.text('Intro'), findsOneWidget);
    expect(find.text('Body'), findsOneWidget, reason: 'the current region is present');
    expect(
      find.byWidgetPredicate(
          (w) => w is Container && (w.decoration as BoxDecoration?)?.color == tokens.good),
      findsOneWidget,
      reason: 'a strong learned cell takes the pass green',
    );
    expect(
      find.byWidgetPredicate(
          (w) => w is Container && (w.decoration as BoxDecoration?)?.color == tokens.again),
      findsOneWidget,
      reason: 'a weak learned cell takes the fail red',
    );
  });

  testWidgets('the current region is emphasized over the others', (tester) async {
    final crumb = ReviewCrumbModel(
      regions: const ['Intro', 'Body'],
      current: 1,
      cells: const [
        ['learned-fading'],
        ['learned-fading'],
      ],
    );
    await pumpStrip(tester, crumb);

    final introStyle = tester.widget<Text>(find.text('Intro')).style!;
    final bodyStyle = tester.widget<Text>(find.text('Body')).style!;

    expect(bodyStyle.fontWeight, FontWeight.w600, reason: 'the current region is bold');
    expect(introStyle.fontWeight, FontWeight.w400, reason: 'the rest stay plain');
    expect(bodyStyle.color!.a, greaterThan(introStyle.color!.a),
        reason: 'the current region is full ink, the rest dimmer');
  });

  testWidgets('a long region path and many cells does not overflow or grow the strip',
      (tester) async {
    final crumb = ReviewCrumbModel(
      regions: List.generate(12, (i) => 'A very long region name number $i that keeps going on'),
      current: 6,
      cells: List.generate(
        12,
        (_) => const [
          'unseen',
          'seen',
          'learning',
          'learned-strong',
          'learned-fading',
          'learned-weak',
          'retired',
        ],
      ),
    );
    await pumpStrip(tester, crumb);

    expect(tester.takeException(), isNull, reason: 'no RenderFlex overflow');
    expect(tester.getSize(find.byType(CrumbStrip)).height, CrumbStrip.height,
        reason: 'the strip stays a fixed height regardless of content');
  });

  Directory tempSupport() {
    final dir = Directory.systemTemp.createTempSync('alix-crumb-support-');
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  Directory plainDeckRoot() {
    final root = Directory.systemTemp.createTempSync('alix-crumb-decks-');
    writeTestDeck('${root.path}/facts.md', '# Facts\n\n## q?\na\n');
    addTearDown(() => root.deleteSync(recursive: true));
    return root;
  }

  testWidgets('a plain non-topology deck: no crumb strip, and review renders normally',
      (tester) async {
    final root = plainDeckRoot();
    await tester.pumpWidget(MaterialApp(
      theme: alixDark(),
      home: ReviewScreen(
        deckPath: '${root.path}/facts.md',
        rootDir: root.path,
        depth: ReviewDepth.recall,
        supportDir: tempSupport(),
      ),
    ));
    await tester.pumpAndSettle();

    expect(find.byType(CrumbStrip), findsNothing,
        reason: 'crumb() is null for a deck with no cached topology');
    expect(find.text('Reveal'), findsOneWidget, reason: 'the review still renders normally');
  });
}
