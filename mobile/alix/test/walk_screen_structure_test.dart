import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/walk_screen.dart';

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

  Future<void> pumpWalk(
    WidgetTester tester, {
    required Directory root,
    Directory? support,
    ServerClient Function(ServerConfig)? buildClient,
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: WalkScreen(
          key: UniqueKey(),
          deckPath: '${root.path}/trace.md',
          rootDir: root.path,
          supportDir: support ?? tempDir('alix-walk-structure-support-'),
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

  testWidgets('walk tree: predict', (tester) async {
    final root = traceRoot('alix-walk-structure-predict-');
    await pumpWalk(tester, root: root);
    await expectWidgetTree(
      tester,
      'walk_predict',
      root: find.byType(WalkScreen),
    );
  });

  testWidgets('walk tree: reveal with excerpt and with excerpt error', (
    tester,
  ) async {
    final root = traceRoot('alix-walk-structure-reveal-');
    await pumpWalk(tester, root: root);
    await reveal(tester);
    await expectWidgetTree(
      tester,
      'walk_reveal_excerpt',
      root: find.byType(WalkScreen),
    );

    final missing = traceRoot(
      'alix-walk-structure-no-source-',
      hops: 1,
      source: false,
    );
    await pumpWalk(tester, root: missing);
    await reveal(tester);
    await expectWidgetTree(
      tester,
      'walk_reveal_excerpt_error',
      root: find.byType(WalkScreen),
    );
  });

  testWidgets('walk tree: done offline, exam available, and exam cooldown', (
    tester,
  ) async {
    final offline = traceRoot('alix-walk-structure-done-offline-', hops: 1);
    await pumpWalk(tester, root: offline);
    await finishOneHop(tester);
    await expectWidgetTree(
      tester,
      'walk_done_offline',
      root: find.byType(WalkScreen),
    );

    final live = traceRoot('alix-walk-structure-done-live-', hops: 1);
    final liveSupport = tempDir('alix-walk-structure-live-support-');
    await setServer(
      const ServerConfig(host: '127.0.0.1', port: 7777, token: 'abc'),
      support: liveSupport,
    );
    await pumpWalk(
      tester,
      root: live,
      support: liveSupport,
      buildClient: (_) => FakeServerClient(versionReply: minServerVersion),
    );
    await finishOneHop(tester);
    await expectWidgetTree(
      tester,
      'walk_done_exam_available',
      root: find.byType(WalkScreen),
    );

    final cooldown = traceRoot('alix-walk-structure-cooldown-', hops: 1);
    final cooldownSupport = tempDir('alix-walk-structure-cooldown-support-');
    await setServer(
      const ServerConfig(host: '127.0.0.1', port: 7777, token: 'abc'),
      support: cooldownSupport,
    );
    WalkSession.open(
      deckPath: '${cooldown.path}/trace.md',
      rootDir: cooldown.path,
    ).applyExamFailed(
      nowMs: BigInt.from(DateTime.now().millisecondsSinceEpoch - 500),
    );
    await pumpWalk(
      tester,
      root: cooldown,
      support: cooldownSupport,
      buildClient: (_) => FakeServerClient(versionReply: minServerVersion),
    );
    await finishOneHop(tester);
    await expectWidgetTree(
      tester,
      'walk_done_exam_cooldown',
      root: find.byType(WalkScreen),
    );
  });

  testWidgets('walk tree: save warning and failed-open feedback', (
    tester,
  ) async {
    final root = traceRoot('alix-walk-structure-save-');
    await pumpWalk(tester, root: root);
    await reveal(tester);
    final progress = File('${root.path}/progress');
    progress.writeAsStringSync('blocks the progress directory');
    await tester.tap(find.text('Got it'));
    await tester.pump();
    progress.deleteSync();
    await tester.pumpAndSettle();
    expect(find.textContaining("Progress isn't being saved"), findsOneWidget);
    await expectWidgetTree(
      tester,
      'walk_save_warning',
      root: find.byType(WalkScreen),
    );

    final invalid = tempDir('alix-walk-structure-invalid-');
    writeTestDeck('${invalid.path}/trace.md', '---\ntitle: Facts\n---\n## q?\na\n');
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: Scaffold(
          body: Builder(
            builder: (context) => FilledButton(
              onPressed: () => Navigator.of(context).push<void>(
                MaterialPageRoute(
                  builder: (_) => WalkScreen(
                    deckPath: '${invalid.path}/trace.md',
                    rootDir: invalid.path,
                    supportDir: tempDir('alix-walk-structure-invalid-support-'),
                  ),
                ),
              ),
              child: const Text('Open invalid walk'),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open invalid walk'));
    await tester.pumpAndSettle();
    expect(find.textContaining('not a trace'), findsOneWidget);
    await expectWidgetTree(
      tester,
      'walk_failed_open_feedback',
      root: find.byType(MaterialApp),
    );
  });
}
