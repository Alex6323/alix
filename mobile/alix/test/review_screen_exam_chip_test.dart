// F48: the review summary has no exam entry. The exam is launched from the
// picker's launch sheet only. ReviewScreen's own
// listing/session calls the real bridge in initState, so RustLib.init() is
// required to mount it, same as review_screen_ask_chip_test.dart (T5).
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

import 'support/deck_fixture.dart';
import 'support/fake_server_client.dart';

void main() {
  setUpAll(() async => RustLib.init());

  Directory tempSupport() {
    final dir = Directory.systemTemp.createTempSync('alix-exam-chip-support-');
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  /// `examinable`: a deck with `% exam:` (never a trace), matching the
  /// bridge's own `has_exam` capture (`!is_trace() && !exam.is_empty()`,
  /// `mobile/alix/rust/src/api/review.rs`). `plain` has no source, so it
  /// never sits an exam.
  Directory deckRoot({required bool examinable}) {
    final root = Directory.systemTemp.createTempSync('alix-exam-chip-decks-');
    final text = examinable
        ? '---\nexam: https://example.com\n---\n## q?\na\n'
        : '## q?\na\n';
    writeTestDeck('${root.path}/facts.md', text);
    addTearDown(() => root.deleteSync(recursive: true));
    return root;
  }

  Future<void> pumpReview(
    WidgetTester tester, {
    required Directory support,
    required bool examinable,
    ServerClient Function(ServerConfig)? buildClient,
  }) async {
    final root = deckRoot(examinable: examinable);
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: ReviewScreen(
          deckPath: '${root.path}/facts.md',
          rootDir: root.path,
          depth: ReviewDepth.recall,
          supportDir: support,
          buildClient: buildClient,
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  // The fixture is a fresh deck's one introduction card: Reveal then Seen
  // finishes the session and lands on the done summary the chip lives on.
  Future<void> finishReview(WidgetTester tester) async {
    await tester.tap(find.text('Reveal'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Seen'));
    await tester.pumpAndSettle();
  }

  testWidgets(
    'an examinable deck with a live paired desktop: the done summary offers no exam chip',
    (tester) async {
      final support = tempSupport();
      await savePairing(
        const ServerConfig(host: '127.0.0.1', port: 7777, token: 'tok', rootId: 'root-test0000000000000000000000'),
        support: support,
      );

      await pumpReview(
        tester,
        support: support,
        examinable: true,
        buildClient: (_) => FakeServerClient(versionReply: '0.6.0'),
      );
      await finishReview(tester);

      expect(find.text('SESSION COMPLETE'), findsOneWidget);
      expect(find.text('Take the exam'), findsNothing);
    },
  );

  testWidgets(
    'a deck with no source: the done summary offers no exam chip either',
    (tester) async {
      final support = tempSupport();
      await savePairing(
        const ServerConfig(host: '127.0.0.1', port: 7777, token: 'tok', rootId: 'root-test0000000000000000000000'),
        support: support,
      );

      await pumpReview(
        tester,
        support: support,
        examinable: false,
        buildClient: (_) => FakeServerClient(versionReply: '0.6.0'),
      );
      await finishReview(tester);

      expect(find.text('SESSION COMPLETE'), findsOneWidget);
      expect(find.text('Take the exam'), findsNothing);
    },
  );
}
