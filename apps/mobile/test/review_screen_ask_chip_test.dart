// The Ask chip's gating: it exists only with a pairing config AND a
// reachable, current-enough desktop AND an attempt made on the current
// card (capability presence, no status chrome; attempt-first, like the
// web client's tutor chip). ReviewScreen's own listing/session calls the
// real bridge in initState, so RustLib.init() is required to mount it,
// same as hashcards_repro_test.dart's own screens.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

/// The probe's test double: a canned `version()` reply, or a thrown
/// PairingExpired (a stale token, refused by an otherwise-live server).
class FakeServerClient implements ServerClient {
  FakeServerClient({this.versionReply, this.expireOnVersion = false});

  final String? versionReply;
  final bool expireOnVersion;
  bool closed = false;

  @override
  Future<String?> version() async {
    if (expireOnVersion) throw const PairingExpired();
    return versionReply;
  }

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
  void close() => closed = true;
}

void main() {
  setUpAll(() async => RustLib.init());

  Directory tempSupport() {
    final dir = Directory.systemTemp.createTempSync('alix-ask-chip-support-');
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  Directory deckRoot() {
    final root = Directory.systemTemp.createTempSync('alix-ask-chip-decks-');
    File('${root.path}/facts.txt').writeAsStringSync('% title: Facts\n# q?\n\ta\n');
    addTearDown(() => root.deleteSync(recursive: true));
    return root;
  }

  Future<void> pumpReview(
    WidgetTester tester, {
    required Directory support,
    ServerClient Function(ServerConfig)? buildClient,
  }) async {
    final root = deckRoot();
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

  // The fixture card is a fresh deck's acquire card, so "an attempt" is
  // the Reveal tap. The negative cases reveal too: without it, they would
  // pass trivially off the attempt gate rather than the server gate.
  Future<void> reveal(WidgetTester tester) async {
    await tester.tap(find.text('Reveal'));
    await tester.pumpAndSettle();
  }

  testWidgets('no pairing config: the Ask chip does not exist', (tester) async {
    await pumpReview(tester, support: tempSupport());
    await reveal(tester);

    expect(find.text('Ask'), findsNothing);
  });

  testWidgets('a paired, current-enough desktop: the Ask chip appears after an attempt', (tester) async {
    final support = tempSupport();
    await setServer(const ServerConfig(host: '127.0.0.1', port: 7777, token: 'tok'), support: support);

    await pumpReview(
      tester,
      support: support,
      buildClient: (_) => FakeServerClient(versionReply: '0.6.0'),
    );

    expect(find.text('Ask'), findsNothing,
        reason: 'attempt-first: no tutor before the learner has tried');
    await reveal(tester);
    expect(find.text('Ask'), findsOneWidget);
  });

  testWidgets('a dead paired server: the Ask chip does not exist', (tester) async {
    final support = tempSupport();
    await setServer(const ServerConfig(host: '127.0.0.1', port: 7777, token: 'tok'), support: support);

    await pumpReview(
      tester,
      support: support,
      buildClient: (_) => FakeServerClient(versionReply: null),
    );
    await reveal(tester);

    expect(find.text('Ask'), findsNothing);
  });

  testWidgets('an older paired server: the Ask chip does not exist', (tester) async {
    final support = tempSupport();
    await setServer(const ServerConfig(host: '127.0.0.1', port: 7777, token: 'tok'), support: support);

    await pumpReview(
      tester,
      support: support,
      buildClient: (_) => FakeServerClient(versionReply: '0.5.0'),
    );
    await reveal(tester);

    expect(find.text('Ask'), findsNothing);
  });

  testWidgets('a refused token (401 on the probe): no chip, and the exact re-pair SnackBar', (tester) async {
    final support = tempSupport();
    await setServer(const ServerConfig(host: '127.0.0.1', port: 7777, token: 'stale'), support: support);

    await pumpReview(
      tester,
      support: support,
      buildClient: (_) => FakeServerClient(expireOnVersion: true),
    );
    await reveal(tester);

    expect(find.text('Ask'), findsNothing);
    expect(
      find.text('Pairing expired. Pair again from the deck list menu.'),
      findsOneWidget,
    );
  });
}
