// The "Take the exam" chip's gating: it exists only on the done summary,
// only with a reachable, current-enough paired desktop AND a deck that
// sits an AI exam (`% source:`, never a trace). ReviewScreen's own
// listing/session calls the real bridge in initState, so RustLib.init() is
// required to mount it, same as review_screen_ask_chip_test.dart (T5).
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

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

  /// `examinable`: a deck with `% source:` (never a trace), matching the
  /// bridge's own `has_exam` capture (`!is_trace() && !sources.is_empty()`,
  /// `apps/mobile/rust/src/api/review.rs`). `plain` has no source, so it
  /// never sits an exam.
  Directory deckRoot({required bool examinable}) {
    final root = Directory.systemTemp.createTempSync('alix-exam-chip-decks-');
    final text = examinable
        ? '---\nsource: https://example.com\n---\n## q?\na\n'
        : '## q?\na\n';
    File('${root.path}/facts.md').writeAsStringSync(text);
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
    await tester.pumpWidget(MaterialApp(
      theme: alixDark(),
      home: ReviewScreen(
        deckPath: '${root.path}/facts.md',
        rootDir: root.path,
        depth: Depth.recall,
        supportDir: support,
        buildClient: buildClient,
      ),
    ));
    await tester.pumpAndSettle();
  }

  // The fixture is a fresh deck's one acquire card: Reveal then Seen
  // finishes the session and lands on the done summary the chip lives on.
  Future<void> finishReview(WidgetTester tester) async {
    await tester.tap(find.text('Reveal'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Seen'));
    await tester.pumpAndSettle();
  }

  testWidgets('an examinable deck with a live paired desktop: the chip appears on the done summary',
      (tester) async {
    final support = tempSupport();
    await setServer(const ServerConfig(host: '127.0.0.1', port: 7777, token: 'tok'), support: support);

    await pumpReview(
      tester,
      support: support,
      examinable: true,
      buildClient: (_) => FakeServerClient(versionReply: '0.6.0'),
    );
    await finishReview(tester);

    expect(find.text('SESSION COMPLETE'), findsOneWidget);
    expect(find.text('Take the exam'), findsOneWidget);
  });

  testWidgets('a deck with no source: the chip does not exist even with a live paired desktop',
      (tester) async {
    final support = tempSupport();
    await setServer(const ServerConfig(host: '127.0.0.1', port: 7777, token: 'tok'), support: support);

    await pumpReview(
      tester,
      support: support,
      examinable: false,
      buildClient: (_) => FakeServerClient(versionReply: '0.6.0'),
    );
    await finishReview(tester);

    expect(find.text('Take the exam'), findsNothing);
  });

  testWidgets('an examinable deck with no pairing config: the chip does not exist', (tester) async {
    await pumpReview(tester, support: tempSupport(), examinable: true);
    await finishReview(tester);

    expect(find.text('Take the exam'), findsNothing);
  });

  // T8.3 regression: review_screen.dart's `_deckName()` (~:210) must resolve
  // a workspace member to `<workspace>/<member>`, the key the desktop's own
  // catalog looks decks up by (`resolve_row`, src/serve/catalog.rs) -- never
  // the device-absolute path, and never a bare basename (a naive
  // `path.basename` would collapse this to `member.md` and pass the chip's
  // own visibility tests above without ever exercising the join).
  testWidgets('a workspace member deck: "Take the exam" starts the sitting on "<workspace>/<member>"',
      (tester) async {
    final root = Directory.systemTemp.createTempSync('alix-exam-chip-ws-decks-');
    addTearDown(() => root.deleteSync(recursive: true));
    Directory('${root.path}/wsfolder').createSync();
    File('${root.path}/wsfolder/alix.toml').writeAsStringSync('title = "Ws"\n');
    File('${root.path}/wsfolder/member.md')
        .writeAsStringSync('---\nsource: https://example.com\n---\n## q?\na\n');

    final support = tempSupport();
    await setServer(const ServerConfig(host: '127.0.0.1', port: 7777, token: 'tok'), support: support);

    final client = FakeServerClient(versionReply: '0.6.0', examStartReply: false);
    await tester.pumpWidget(MaterialApp(
      theme: alixDark(),
      home: ReviewScreen(
        deckPath: '${root.path}/wsfolder/member.md',
        rootDir: root.path,
        depth: Depth.recall,
        supportDir: support,
        buildClient: (_) => client,
      ),
    ));
    await tester.pumpAndSettle();
    await finishReview(tester);

    expect(find.text('Take the exam'), findsOneWidget);
    await tester.tap(find.text('Take the exam'));
    await tester.pumpAndSettle();

    expect(client.startedDeck, 'wsfolder/member.md');
  });
}
