import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/trace_screen.dart';

import 'support/deck_fixture.dart';
import 'support/fake_server_client.dart';
import 'support/widget_tree_dump.dart';

void main() {
  setUpAll(() async => RustLib.init());

  Directory tempDir(String prefix) {
    final dir = Directory.systemTemp.createTempSync(prefix);
    addTearDown(() {
      if (dir.existsSync()) {
        if (Platform.isLinux || Platform.isMacOS) {
          Process.runSync('chmod', ['-R', 'u+rwx', dir.path]);
        }
        dir.deleteSync(recursive: true);
      }
    });
    return dir;
  }

  Directory traceRoot(String prefix, {int hops = 2, bool source = true}) {
    final root = tempDir(prefix);
    if (source) {
      File(
        '${root.path}/source.txt',
      ).writeAsStringSync('first\nsecond\nthird\n');
    }
    final second = hops == 1
        ? ''
        : '\n## Predict the second hop\n'
              'it reads lines two and three\n'
              '<!-- at: 2-3 -->\n';
    writeTestDeck(
      '${root.path}/trace.md',
      '---\ntrace: how it works${source ? '\nsource: source.txt' : ''}\n---\n'
          '## Predict the first hop\n'
          'it reads the first line\n'
          '<!-- at: 1 -->\n'
          '$second',
    );
    return root;
  }

  Future<void> pumpTraceSession(
    WidgetTester tester, {
    required Directory root,
    Directory? support,
    ServerClient Function(ServerConfig)? buildClient,
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: TraceSessionScreen(
          key: UniqueKey(),
          deckPath: '${root.path}/trace.md',
          rootDir: root.path,
          supportDir: support ?? tempDir('alix-trace-structure-support-'),
          buildClient: buildClient,
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> reveal(WidgetTester tester, {String guess = 'a guess'}) async {
    await tester.enterText(find.byType(TextField), guess);
    await tester.tap(find.text('Reveal'));
    await tester.pumpAndSettle();
  }

  Future<void> finishOneHop(WidgetTester tester) async {
    await reveal(tester);
    await tester.tap(find.text('Got it'));
    await tester.pumpAndSettle();
  }

  testWidgets('trace tree: predict', (tester) async {
    final root = traceRoot('alix-trace-structure-predict-');
    await pumpTraceSession(tester, root: root);
    await expectWidgetTree(
      tester,
      'trace_predict',
      root: find.byType(TraceSessionScreen),
    );
  });

  testWidgets('trace tree: reveal with excerpt and with excerpt error', (
    tester,
  ) async {
    final root = traceRoot('alix-trace-structure-reveal-');
    await pumpTraceSession(tester, root: root);
    await reveal(tester);
    await expectWidgetTree(
      tester,
      'trace_reveal_excerpt',
      root: find.byType(TraceSessionScreen),
    );

    final missing = traceRoot(
      'alix-trace-structure-no-source-',
      hops: 1,
      source: false,
    );
    await pumpTraceSession(tester, root: missing);
    await reveal(tester);
    await expectWidgetTree(
      tester,
      'trace_reveal_excerpt_error',
      root: find.byType(TraceSessionScreen),
    );
  });

  testWidgets('trace tree: done offline, exam available, and exam cooldown', (
    tester,
  ) async {
    final offline = traceRoot('alix-trace-structure-done-offline-', hops: 1);
    await pumpTraceSession(tester, root: offline);
    await finishOneHop(tester);
    await expectWidgetTree(
      tester,
      'trace_done_offline',
      root: find.byType(TraceSessionScreen),
    );

    final live = traceRoot('alix-trace-structure-done-live-', hops: 1);
    final liveSupport = tempDir('alix-trace-structure-live-support-');
    await savePairing(
      const ServerConfig(host: '127.0.0.1', port: 7777, token: 'abc', rootId: 'root-test0000000000000000000000'),
      support: liveSupport,
    );
    await pumpTraceSession(
      tester,
      root: live,
      support: liveSupport,
      buildClient: (_) => FakeServerClient(versionReply: minServerVersion),
    );
    await finishOneHop(tester);
    await expectWidgetTree(
      tester,
      'trace_done_exam_available',
      root: find.byType(TraceSessionScreen),
    );

    final cooldown = traceRoot('alix-trace-structure-cooldown-', hops: 1);
    final cooldownSupport = tempDir('alix-trace-structure-cooldown-support-');
    await savePairing(
      const ServerConfig(host: '127.0.0.1', port: 7777, token: 'abc', rootId: 'root-test0000000000000000000000'),
      support: cooldownSupport,
    );
    TraceSession.open(
      deckPath: '${cooldown.path}/trace.md',
      rootDir: cooldown.path,
    ).applyExamFailed(
      nowMs: BigInt.from(DateTime.now().millisecondsSinceEpoch - 500),
    );
    await pumpTraceSession(
      tester,
      root: cooldown,
      support: cooldownSupport,
      buildClient: (_) => FakeServerClient(versionReply: minServerVersion),
    );
    await finishOneHop(tester);
    await expectWidgetTree(
      tester,
      'trace_done_exam_cooldown',
      root: find.byType(TraceSessionScreen),
    );
  });

  testWidgets('trace tree: save warning and failed-open feedback', (
    tester,
  ) async {
    final root = traceRoot('alix-trace-structure-save-');
    await pumpTraceSession(tester, root: root);
    await reveal(tester);
    Directory('${root.path}/.alix').createSync();
    final progress = File('${root.path}/.alix/progress');
    progress.writeAsStringSync('blocks the progress directory');
    await tester.tap(find.text('Got it'));
    await tester.pump();
    progress.deleteSync();
    await tester.pumpAndSettle();
    expect(find.textContaining("Progress isn't being saved"), findsOneWidget);
    await expectWidgetTree(
      tester,
      'trace_save_warning',
      root: find.byType(TraceSessionScreen),
    );

    final invalid = tempDir('alix-trace-structure-invalid-');
    writeTestDeck('${invalid.path}/trace.md', '---\ntitle: Facts\n---\n## q?\na\n');
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: Scaffold(
          body: Builder(
            builder: (context) => FilledButton(
              onPressed: () => Navigator.of(context).push<void>(
                MaterialPageRoute(
                  builder: (_) => TraceSessionScreen(
                    deckPath: '${invalid.path}/trace.md',
                    rootDir: invalid.path,
                    supportDir: tempDir('alix-trace-structure-invalid-support-'),
                  ),
                ),
              ),
              child: const Text('Open invalid trace'),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open invalid trace'));
    await tester.pumpAndSettle();
    expect(find.textContaining('not a trace'), findsOneWidget);
    await expectWidgetTree(
      tester,
      'trace_failed_open_feedback',
      root: find.byType(MaterialApp),
    );
  });
}
