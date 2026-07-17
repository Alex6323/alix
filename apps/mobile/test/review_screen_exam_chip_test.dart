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

/// The probe's test double: a canned `version()` reply. The exam surface
/// itself is never exercised by these tests (the chip only needs to exist,
/// not be tapped), so every exam/tutor method is an unreached stub.
class FakeServerClient implements ServerClient {
  FakeServerClient({this.versionReply});

  final String? versionReply;

  @override
  Future<String?> version() async => versionReply;

  @override
  Future<String?> backendName() async => null;

  @override
  Future<bool> postAsk(TutorCardContext card, List<TutorTurn> history, String question) async => false;

  @override
  Future<RemoteAsk?> getAsk() async => null;

  @override
  Future<bool> postDraft(TutorCardContext card, List<TutorTurn> history) async => false;

  @override
  Future<bool> postNote(TutorCardContext card, List<TutorTurn> history) async => false;

  @override
  Future<bool> examStart(String deck) async => false;

  @override
  Future<RemoteExam?> examGet() async => null;

  @override
  Future<bool> examGrade(List<String> answers) async => false;

  @override
  Future<bool> examRemediate() async => false;

  @override
  Future<void> examClose() async {}

  @override
  Future<bool> generateStart(String url, {String? guidance}) async => false;

  @override
  Future<RemoteGenerate?> generateGet() async => null;

  @override
  Future<void> generateClose() async {}

  @override
  void close() {}
}

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
        ? '% source: https://example.com\n# q?\n\ta\n'
        : '# q?\n\ta\n';
    File('${root.path}/facts.txt').writeAsStringSync(text);
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
        deckPath: '${root.path}/facts.txt',
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
}
