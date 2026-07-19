// Leaving a review with cards still due asks for confirmation first (a stray
// back swipe shouldn't abandon a session), while a finished session leaves at
// once.
import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';

void main() {
  setUpAll(() async => RustLib.init());

  Directory dueDeck() {
    final root = Directory.systemTemp.createTempSync('alix-leave-');
    File('${root.path}/facts.md').writeAsStringSync(
        '# Facts\n\n## q? <!-- id: q1 -->\na\n\n## q2? <!-- id: q2 -->\nb\n');
    addTearDown(() => root.deleteSync(recursive: true));
    return root;
  }

  Directory support() {
    final dir = Directory.systemTemp.createTempSync('alix-leave-support-');
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  // Pushes a ReviewScreen so it has a route below it (an AppBar back button,
  // and a system back that can pop), then settles.
  Future<void> pushReview(WidgetTester tester, Directory root) async {
    await tester.pumpWidget(const MaterialApp(home: Scaffold()));
    final context = tester.element(find.byType(Scaffold));
    unawaited(Navigator.of(context).push(MaterialPageRoute(
      builder: (_) => ReviewScreen(
        deckPath: '${root.path}/facts.md',
        rootDir: root.path,
        depth: Depth.recall,
        supportDir: support(),
      ),
    )));
    await tester.pumpAndSettle();
  }

  testWidgets('back with cards due asks first; Keep reviewing stays',
      (tester) async {
    await pushReview(tester, dueDeck());
    expect(find.byType(ReviewScreen), findsOneWidget);

    await tester.tap(find.byType(BackButton));
    await tester.pumpAndSettle();
    expect(find.text('Leave the review?'), findsOneWidget,
        reason: 'a fresh deck has due cards, so leaving is confirmed');

    await tester.tap(find.text('Keep reviewing'));
    await tester.pumpAndSettle();
    expect(find.text('Leave the review?'), findsNothing);
    expect(find.byType(ReviewScreen), findsOneWidget,
        reason: 'Keep reviewing dismisses the dialog and stays');
  });

  testWidgets('Leave confirms out of the review', (tester) async {
    await pushReview(tester, dueDeck());

    await tester.tap(find.byType(BackButton));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Leave'));
    await tester.pumpAndSettle();
    expect(find.byType(ReviewScreen), findsNothing,
        reason: 'Leave pops the review');
  });
}
