// Widget tests for the generate sheet (PickerScreen's overflow menu ->
// "Generate deck..."): the pairing-gated menu item, the local http(s)-only
// URL gate, the poll -> dest pick -> apply_generated_deck happy path, and
// the error/cancel paths' generateClose bookkeeping. Driven with an injected
// fake ServerClient, a real temp support Directory (settings.json), and a
// real temp decks Directory: PickerScreen's own listing and the bridge's
// apply_generated_deck both call the real embedded core, so RustLib.init()
// is required to mount anything here, same as pairing_sheet_test.dart's own
// screens.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';

/// The generate sheet's test double: canned replies for generateStart/Get,
/// and every call recorded so the LOCKED-refusal and generateClose tests can
/// assert on it directly.
class FakeServerClient implements ServerClient {
  FakeServerClient({
    this.startReply = true,
    this.expireOnStart = false,
    List<RemoteGenerate>? getReplies,
    this.expireOnGet = false,
  }) : getReplies = getReplies ?? const [];

  final bool startReply;
  final bool expireOnStart;
  final List<RemoteGenerate> getReplies;
  final bool expireOnGet;

  bool startCalled = false;
  String? startedUrl;
  String? startedGuidance;
  int closeCalls = 0;
  bool closed = false;
  int _getCall = 0;

  @override
  Future<bool> generateStart(String url, {String? guidance}) async {
    startCalled = true;
    startedUrl = url;
    startedGuidance = guidance;
    if (expireOnStart) throw const PairingExpired();
    return startReply;
  }

  @override
  Future<RemoteGenerate?> generateGet() async {
    if (expireOnGet) throw const PairingExpired();
    if (getReplies.isEmpty) return null;
    final call = _getCall;
    _getCall++;
    return getReplies[call.clamp(0, getReplies.length - 1)];
  }

  @override
  Future<void> generateClose() async => closeCalls++;

  @override
  Future<String?> version() async => null;

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
  void close() => closed = true;
}

void main() {
  setUpAll(() async => RustLib.init());

  Directory temp(String prefix) {
    final dir = Directory.systemTemp.createTempSync(prefix);
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  Future<void> pair(Directory support) => setServer(
        const ServerConfig(host: 'desktop.local', port: 7777, token: 'abc123'),
        support: support,
      );

  Future<void> openPicker(
    WidgetTester tester, {
    required String root,
    required Directory support,
    required ServerClient Function(ServerConfig) buildClient,
  }) async {
    await tester.pumpWidget(MaterialApp(
      home: PickerScreen(
        root: root,
        onSetDecksDir: (_) async {},
        supportDir: support,
        buildClient: buildClient,
        generatePollInterval: const Duration(milliseconds: 10),
      ),
    ));
    await tester.pumpAndSettle();
  }

  Future<void> openGenerateSheet(WidgetTester tester) async {
    await tester.tap(find.byType(PopupMenuButton<String>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Generate deck…'));
    await tester.pumpAndSettle();
  }

  testWidgets('the Generate deck… item is absent when unpaired', (tester) async {
    final support = temp('alix-gen-support-');
    final root = temp('alix-gen-decks-');
    await openPicker(tester, root: root.path, support: support, buildClient: (_) => FakeServerClient());

    await tester.tap(find.byType(PopupMenuButton<String>));
    await tester.pumpAndSettle();

    expect(find.text('Generate deck…'), findsNothing);
  });

  testWidgets('the Generate deck… item appears when paired', (tester) async {
    final support = temp('alix-gen-support-');
    final root = temp('alix-gen-decks-');
    await pair(support);
    await openPicker(tester, root: root.path, support: support, buildClient: (_) => FakeServerClient());

    await tester.tap(find.byType(PopupMenuButton<String>));
    await tester.pumpAndSettle();

    expect(find.text('Generate deck…'), findsOneWidget);
  });

  testWidgets('a non-http URL is refused locally: no generateStart call, a calm message shown', (tester) async {
    final support = temp('alix-gen-support-');
    final root = temp('alix-gen-decks-');
    await pair(support);
    final client = FakeServerClient();
    await openPicker(tester, root: root.path, support: support, buildClient: (_) => client);
    await openGenerateSheet(tester);

    await tester.enterText(find.byKey(const ValueKey('generate-url-field')), 'file:///x');
    await tester.tap(find.text('Generate'));
    await tester.pumpAndSettle();

    expect(client.startCalled, isFalse);
    expect(find.textContaining('http:// or https://'), findsOneWidget);
  });

  testWidgets(
      'happy path: generateStart -> a done DTO -> the dest picker -> apply_generated_deck places the '
      'file, SnackBar + refresh', (tester) async {
    final support = temp('alix-gen-support-');
    final root = temp('alix-gen-decks-');
    await pair(support);
    const deckText = '# capital of testland?\n\tTestville\n';
    final client = FakeServerClient(
      getReplies: const [
        RemoteGenerate(phase: 'done', deck: deckText, filename: 'generated.txt', cards: 1),
      ],
    );
    await openPicker(tester, root: root.path, support: support, buildClient: (_) => client);
    await openGenerateSheet(tester);

    await tester.enterText(find.byKey(const ValueKey('generate-url-field')), 'https://example.org/article');
    await tester.enterText(find.byKey(const ValueKey('generate-guidance-field')), '  focus on basics  ');
    await tester.tap(find.text('Generate'));
    await tester.pumpAndSettle();

    expect(client.startedUrl, 'https://example.org/article');
    expect(client.startedGuidance, 'focus on basics');

    // The dest picker (FolderBrowser) opened at the decks root; save there.
    expect(find.text('Use this folder'), findsOneWidget);
    await tester.tap(find.text('Use this folder'));
    await tester.pumpAndSettle();

    final written = File('${root.path}/generated.txt');
    expect(written.existsSync(), isTrue, reason: 'the phone, not the desktop, places the file');
    expect(written.readAsStringSync(), contains('Testville'));

    expect(find.textContaining('saved as generated.txt'), findsOneWidget);
    expect(client.closeCalls, 1, reason: 'the server generation slot is freed once placed');
    expect(client.closed, isTrue);

    // The picker refreshed: the newly placed deck's row is now visible
    // (its title defaults to the file stem, no `% title:` in the fixture).
    expect(find.text('generated'), findsOneWidget);
  });

  testWidgets('a "generating" poll tick shows the elapsed seconds, then the next tick lands on done',
      (tester) async {
    final support = temp('alix-gen-support-');
    final root = temp('alix-gen-decks-');
    await pair(support);
    const deckText = '# q\n\ta\n';
    final client = FakeServerClient(
      getReplies: const [
        RemoteGenerate(phase: 'generating', elapsed: 2),
        RemoteGenerate(phase: 'done', deck: deckText, filename: 'x.txt', cards: 1),
      ],
    );
    await openPicker(tester, root: root.path, support: support, buildClient: (_) => client);
    await openGenerateSheet(tester);

    await tester.enterText(find.byKey(const ValueKey('generate-url-field')), 'https://example.org');
    await tester.tap(find.text('Generate'));
    // One pump each for the tap's own frame and for generateStart's and the
    // first generateGet's resolved microtasks, landing on the "generating"
    // DTO without yet advancing the poll timer.
    await tester.pump();
    await tester.pump();
    await tester.pump();

    expect(find.textContaining('The desktop is working… 2s'), findsOneWidget);
    expect(find.text('Use this folder'), findsNothing, reason: 'still generating, not yet done');

    // The next tick of the poll timer lands on the queued "done" DTO.
    await tester.pump(const Duration(milliseconds: 10));
    await tester.pumpAndSettle();

    expect(find.text('Use this folder'), findsOneWidget);
  });

  testWidgets('cancelling the dest pick saves nothing and still frees the server slot', (tester) async {
    final support = temp('alix-gen-support-');
    final root = temp('alix-gen-decks-');
    await pair(support);
    const deckText = '# q\n\ta\n';
    final client = FakeServerClient(
      getReplies: const [RemoteGenerate(phase: 'done', deck: deckText, filename: 'x.txt', cards: 1)],
    );
    await openPicker(tester, root: root.path, support: support, buildClient: (_) => client);
    await openGenerateSheet(tester);

    await tester.enterText(find.byKey(const ValueKey('generate-url-field')), 'https://example.org');
    await tester.tap(find.text('Generate'));
    await tester.pumpAndSettle();

    expect(find.text('Use this folder'), findsOneWidget);
    await tester.tap(find.byTooltip('Back'));
    await tester.pumpAndSettle();

    expect(root.listSync(), isEmpty, reason: 'nothing is saved when the dest pick is cancelled');
    expect(client.closeCalls, 1);
  });

  testWidgets('an error-phase DTO shows the message and places no file', (tester) async {
    final support = temp('alix-gen-support-');
    final root = temp('alix-gen-decks-');
    await pair(support);
    final client = FakeServerClient(
      getReplies: const [RemoteGenerate(phase: 'error', error: 'the model returned no deck content')],
    );
    await openPicker(tester, root: root.path, support: support, buildClient: (_) => client);
    await openGenerateSheet(tester);

    await tester.enterText(find.byKey(const ValueKey('generate-url-field')), 'https://example.org');
    await tester.tap(find.text('Generate'));
    await tester.pumpAndSettle();

    expect(find.text('the model returned no deck content'), findsOneWidget);
    expect(root.listSync(), isEmpty);

    // The sheet stays open on a failure (matching the pairing sheet's
    // idiom); dismissing it is what frees the slot.
    await tester.tapAt(const Offset(20, 20));
    await tester.pumpAndSettle();

    expect(client.closeCalls, 1);
  });

  testWidgets('generateClose is called when the sheet is dismissed before ever submitting (cancel path)',
      (tester) async {
    final support = temp('alix-gen-support-');
    final root = temp('alix-gen-decks-');
    await pair(support);
    final client = FakeServerClient();
    await openPicker(tester, root: root.path, support: support, buildClient: (_) => client);
    await openGenerateSheet(tester);

    await tester.tapAt(const Offset(20, 20));
    await tester.pumpAndSettle();

    expect(client.startCalled, isFalse);
    expect(client.closeCalls, 1);
  });
}
