// The reveal can grow the card below the fold (the note especially); the
// quiet "more" pill must say so, and get out of the way at the bottom.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/review_card.dart';

void main() {
  Widget host({required double childHeight}) {
    return MaterialApp(
      home: Scaffold(
        body: SizedBox(
          height: 300,
          child: ScrollWithMoreHint(
            child: SingleChildScrollView(
              primary: true,
              child: SizedBox(height: childHeight, width: 100),
            ),
          ),
        ),
      ),
    );
  }

  double pillOpacity(WidgetTester tester) {
    final animated = tester.widget<AnimatedOpacity>(
      find.ancestor(
        of: find.text('⌵ more'),
        matching: find.byType(AnimatedOpacity),
      ),
    );
    return animated.opacity;
  }

  testWidgets('content that fits shows no more pill', (tester) async {
    await tester.pumpWidget(host(childHeight: 100));
    await tester.pumpAndSettle();
    expect(pillOpacity(tester), 0,
        reason: 'nothing is hidden, so nothing to announce');
  });

  testWidgets('content below the fold shows the pill until the bottom',
      (tester) async {
    await tester.pumpWidget(host(childHeight: 900));
    await tester.pumpAndSettle();
    expect(pillOpacity(tester), 1,
        reason: 'hidden content below must be announced');

    await tester.drag(find.byType(SingleChildScrollView), const Offset(0, -900));
    await tester.pumpAndSettle();
    expect(pillOpacity(tester), 0,
        reason: 'at the bottom there is nothing more to see');
  });

  testWidgets('growing content brings the pill back, like a reveal',
      (tester) async {
    await tester.pumpWidget(host(childHeight: 100));
    await tester.pumpAndSettle();
    expect(pillOpacity(tester), 0, reason: 'fits before the growth');

    await tester.pumpWidget(host(childHeight: 900));
    await tester.pumpAndSettle();
    expect(pillOpacity(tester), 1,
        reason: 'growth past the fold must re-announce');
  });
}
