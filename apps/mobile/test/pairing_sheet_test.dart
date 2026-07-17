// Widget tests for the pairing sheet (PickerScreen's overflow menu ->
// "Pair with desktop..."), driven with an injected fake ServerClient and a
// real temp support Directory (settings.json). PickerScreen's own listing
// calls the real bridge in initState, so RustLib.init() is required to
// mount it at all, same as bridge_test.dart's own screens.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';

/// The pairing probe's test double: no network, a canned version() reply
/// (or a thrown PairingExpired, modelling a stale pasted token).
class FakeServerClient implements ServerClient {
  FakeServerClient({this.versionReply, this.expiredOnVersion = false});

  final String? versionReply;
  final bool expiredOnVersion;
  bool closed = false;

  @override
  Future<String?> version() async {
    if (expiredOnVersion) throw const PairingExpired();
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

  Directory temp(String prefix) {
    final dir = Directory.systemTemp.createTempSync(prefix);
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  Directory decksRoot() {
    final root = Directory.systemTemp.createTempSync('alix-pairing-decks-');
    addTearDown(() => root.deleteSync(recursive: true));
    return root;
  }

  Future<void> openPairSheet(
    WidgetTester tester, {
    required Directory support,
    required ServerClient Function(ServerConfig) buildClient,
  }) async {
    await tester.pumpWidget(MaterialApp(
      home: PickerScreen(
        root: decksRoot().path,
        onSetDecksDir: (_) async {},
        supportDir: support,
        buildClient: buildClient,
      ),
    ));
    await tester.pumpAndSettle();
    await tester.tap(find.byType(PopupMenuButton<String>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Pair with desktop…'));
    await tester.pumpAndSettle();
  }

  testWidgets('an unparsable paste shows an inline parse-error message', (tester) async {
    final support = temp('alix-support-');
    await openPairSheet(
      tester,
      support: support,
      buildClient: (_) => FakeServerClient(versionReply: '0.6.0'),
    );

    await tester.enterText(find.byKey(const ValueKey('pairing-url-field')), 'not a url at all');
    await tester.tap(find.text('Pair'));
    await tester.pumpAndSettle();

    expect(find.textContaining('does not look like an alix pairing URL'), findsOneWidget);
  });

  testWidgets('no reply from the probe shows the no-alix inline message', (tester) async {
    final support = temp('alix-support-');
    await openPairSheet(
      tester,
      support: support,
      buildClient: (_) => FakeServerClient(versionReply: null),
    );

    await tester.enterText(
      find.byKey(const ValueKey('pairing-url-field')),
      'http://192.168.1.9:7777/?token=abc123',
    );
    await tester.tap(find.text('Pair'));
    await tester.pumpAndSettle();

    expect(find.textContaining('no alix answered at 192.168.1.9:7777'), findsOneWidget);
  });

  testWidgets('a refused token shows its own inline message and persists nothing', (tester) async {
    final support = temp('alix-support-');
    await openPairSheet(
      tester,
      support: support,
      buildClient: (_) => FakeServerClient(expiredOnVersion: true),
    );

    await tester.enterText(
      find.byKey(const ValueKey('pairing-url-field')),
      'http://192.168.1.9:7777/?token=stale00',
    );
    await tester.tap(find.text('Pair'));
    await tester.pumpAndSettle();

    expect(
      find.text('alix answered but refused this token. '
          'Copy a fresh pairing URL from the server.'),
      findsOneWidget,
    );
    expect(readSettings(support)['server'], isNull, reason: 'a refused token must not persist');
  });

  testWidgets('an older server version shows the too-old inline message', (tester) async {
    final support = temp('alix-support-');
    await openPairSheet(
      tester,
      support: support,
      buildClient: (_) => FakeServerClient(versionReply: '0.5.0'),
    );

    await tester.enterText(
      find.byKey(const ValueKey('pairing-url-field')),
      'http://192.168.1.9:7777/?token=abc123',
    );
    await tester.tap(find.text('Pair'));
    await tester.pumpAndSettle();

    expect(
      find.textContaining('alix 0.5.0 found, this app needs 0.6.0 or newer'),
      findsOneWidget,
    );
  });

  testWidgets('a successful pair persists the config and shows a SnackBar', (tester) async {
    final support = temp('alix-support-');
    await openPairSheet(
      tester,
      support: support,
      buildClient: (_) => FakeServerClient(versionReply: '0.6.0'),
    );

    await tester.enterText(
      find.byKey(const ValueKey('pairing-url-field')),
      'http://desktop.local:7777/?token=abc123',
    );
    await tester.tap(find.text('Pair'));
    await tester.pumpAndSettle();

    expect(find.textContaining('Paired with desktop.local'), findsOneWidget);

    final saved = ServerConfig.fromJson(readSettings(support)['server']);
    expect(saved, const ServerConfig(host: 'desktop.local', port: 7777, token: 'abc123'));
  });

  testWidgets('Unpair removes the setting and shows a SnackBar', (tester) async {
    final support = temp('alix-support-');
    const config = ServerConfig(host: 'desktop.local', port: 7777, token: 'abc123');
    await setServer(config, support: support);

    await openPairSheet(
      tester,
      support: support,
      buildClient: (_) => FakeServerClient(versionReply: '0.6.0'),
    );

    expect(find.textContaining('Paired with desktop.local:7777'), findsOneWidget);
    await tester.tap(find.text('Unpair'));
    await tester.pumpAndSettle();

    expect(find.textContaining('Unpaired'), findsOneWidget);
    final raw = jsonDecode(File('${support.path}/settings.json').readAsStringSync()) as Map;
    expect(raw.containsKey('server'), isFalse);
  });
}
