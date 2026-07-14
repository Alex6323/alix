// Tier-2 mobile e2e: the full app widget tree on a REAL device target (the
// Linux desktop window in CI, an emulator for the on-Android tier), driving
// picker -> depth pick -> review -> grade against the real core and
// asserting the store file. The acquire is backdated through the bridge so
// the wall-clock UI serves a quiz immediately; nothing sleeps.
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

import 'package:alix_mobile/main.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('pick a deck, review a due card, the grade lands in the store',
      (tester) async {
    await RustLib.init();
    final root = Directory.systemTemp.createTempSync('alix-e2e-');
    addTearDown(() => root.deleteSync(recursive: true));
    File('${root.path}/greek.txt').writeAsStringSync(
      '% title: Greek\n# capital of greece?\n    Athens\n',
    );
    // Acquired two minutes "ago": the app, on the wall clock, serves the quiz.
    final backdated =
        BigInt.from(DateTime.now().millisecondsSinceEpoch - 120000);
    ReviewSession.open(
      deckPath: '${root.path}/greek.txt',
      rootDir: root.path,
      nowMs: backdated,
    ).acquire(nowMs: backdated);

    await tester.pumpWidget(AlixApp(root: root.path));
    await tester.pumpAndSettle();
    expect(find.text('Greek'), findsOneWidget);

    await tester.tap(find.text('Greek'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Recall'));
    await tester.pumpAndSettle();

    expect(find.text('capital of greece?'), findsOneWidget);
    await tester.tap(find.text('Reveal'));
    await tester.pumpAndSettle();
    expect(find.text('Athens'), findsOneWidget);
    await tester.tap(find.text('Pass'));
    await tester.pumpAndSettle();
    expect(find.text('Done for now'), findsOneWidget);

    final store = File('${root.path}/progress.json').readAsStringSync();
    expect(store, contains('"stability"'));
    expect(store, contains('"history"'));
  });
}
