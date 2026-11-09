import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/trace_screen.dart';

import 'support/deck_fixture.dart';
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
              '<!-- at: src:source.txt:2-3 -->\n';
    writeTestDeck(
      '${root.path}/trace.md',
      '---\ntrace: how it works'
          '${source ? '\ncite:\n  - src: .' : ''}\n---\n'
          '## Predict the first hop\n'
          'it reads the first line\n'
          '<!-- at: src:source.txt:1 -->\n'
          '$second',
    );
    return root;
  }

  Future<void> pumpTraceSession(
    WidgetTester tester, {
    required Directory root,
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: alixDark(),
        home: TraceSessionScreen(
          key: UniqueKey(),
          deckPath: '${root.path}/trace.md',
          rootDir: root.path,
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

  testWidgets('trace tree: done', (tester) async {
    final offline = traceRoot('alix-trace-structure-done-offline-', hops: 1);
    await pumpTraceSession(tester, root: offline);
    await finishOneHop(tester);
    await expectWidgetTree(
      tester,
      'trace_done_offline',
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
