import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

import 'support/deck_fixture.dart';

const _longTitle =
    'A deck title long enough that one phone app bar line cannot hold it';

const _flip =
    '---\ntitle: $_longTitle\n---\n'
    '## capital of italy?\nRome\n';

const _choice =
    '---\ntitle: $_longTitle\n---\n'
    '## capital of france?\n- [x] Paris\n- [ ] London\n- [ ] Berlin\n'
    '<!-- choices: single -->\n';

const _modeWords = [
  'NEW',
  'FLIP',
  'CHOICE',
  'SELECT ALL',
  'TYPING',
  'TYPING · LINE',
  'EXPLAIN',
  'LINE',
];

const _advance = ['Reveal', 'Seen', 'Got it', 'Next'];

void main() {
  setUpAll(() async => RustLib.init());

  final title = find.byKey(const ValueKey('review-title'));
  final cramCaption = find.byKey(const ValueKey('new-session-cram'));
  Finder chip(String label) => find.widgetWithText(InkWell, label);

  Future<String> deckIn(String name, String text, {bool due = false}) async {
    final root = Directory.systemTemp.createTempSync('alix-review-chrome-');
    addTearDown(() => root.deleteSync(recursive: true));
    final deck = '${root.path}/$name.md';
    writeTestDeck(deck, text);
    if (due) {
      final then = BigInt.from(
        DateTime.now().millisecondsSinceEpoch -
            const Duration(minutes: 10).inMilliseconds,
      );
      final session = ReviewSession.open(
        deckPath: deck,
        rootDir: root.path,
        nowMs: then,
      );
      while (session.state(nowMs: then).introducing) {
        session.introduce(nowMs: then);
      }
    }
    return deck;
  }

  Future<void> pumpReview(
    WidgetTester tester,
    String deck, {
    required ReviewDepth depth,
    required bool cram,
  }) async {
    final support = Directory.systemTemp.createTempSync('alix-chrome-sup-');
    addTearDown(() => support.deleteSync(recursive: true));
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: ReviewScreen(
          key: UniqueKey(),
          deckPath: deck,
          rootDir: File(deck).parent.path,
          depth: depth,
          cram: cram,
          supportDir: support,
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  void expectChrome(WidgetTester tester, String step, {required bool cram}) {
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
    expect(
      find.byType(AlixWordmark),
      findsNothing,
      reason: '$step: no wordmark',
    );
    for (final word in _modeWords) {
      expect(find.text(word), findsNothing, reason: '$step: no "$word" tag');
    }
    expect(
      find.textContaining('new card:'),
      findsNothing,
      reason: '$step: no new-card line',
    );
    final summary = chip('New session').evaluate().isNotEmpty;
    expect(
      cramCaption.evaluate().length,
      summary && cram ? 1 : 0,
      reason: '$step: the cram caption shows iff summary ($summary) and cram',
    );
  }

  Future<List<String>> sweep(
    WidgetTester tester, {
    required String label,
    required bool cram,
  }) async {
    final steps = <String>[];
    for (var turn = 0; chip('New session').evaluate().isEmpty; turn++) {
      expect(turn, lessThan(8), reason: '$label: the session ends');
      final step = '$label step $turn';
      expectChrome(tester, step, cram: cram);
      final next = _advance.where((l) => chip(l).evaluate().isNotEmpty);
      if (next.isNotEmpty) {
        steps.add(next.first);
        await tester.ensureVisible(chip(next.first));
        await tester.tap(chip(next.first));
      } else {
        expect(find.text('Paris'), findsOneWidget, reason: '$step: a pick');
        steps.add('Paris');
        await tester.tap(find.text('Paris'));
      }
      await tester.pumpAndSettle();
    }
    expectChrome(tester, '$label summary', cram: cram);
    return steps;
  }

  testWidgets('law: every review step names the deck on one truncated line, '
      'shows no mode tag or new-card line, and captions cram only on a '
      'cram summary', (tester) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);
    for (final cram in [false, true]) {
      final runs = {
        'fresh flip': (await deckIn('flip', _flip), ReviewDepth.recall),
        'due flip': (
          await deckIn('flip', _flip, due: true),
          ReviewDepth.recall,
        ),
        'fresh choice': (await deckIn('pick', _choice), ReviewDepth.recall),
        'due choice': (
          await deckIn('pick', _choice, due: true),
          ReviewDepth.recognize,
        ),
      };
      final walked = <String, List<String>>{};
      for (final MapEntry(key: name, value: (deck, depth)) in runs.entries) {
        await pumpReview(tester, deck, depth: depth, cram: cram);
        final label = '$name, cram $cram';
        walked[name] = await sweep(tester, label: label, cram: cram);
      }
      expect(walked, {
        'fresh flip': ['Reveal', 'Seen'],
        'due flip': ['Reveal', 'Got it'],
        'fresh choice': ['Paris', 'Seen'],
        'due choice': ['Paris', 'Next'],
      }, reason: 'cram $cram: each run crosses its front and answer');
    }
  });
}
