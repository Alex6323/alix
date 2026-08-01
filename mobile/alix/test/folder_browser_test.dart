import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/folder_browser.dart';
import 'package:alix_mobile/theme.dart';

void main() {
  group('parentOf', () {
    test('is null at the floor', () {
      expect(parentOf('/storage/emulated/0', '/storage/emulated/0'), isNull);
    });
    test('is the parent one level below the floor', () {
      expect(parentOf('/storage/emulated/0/Sync', '/storage/emulated/0'),
          '/storage/emulated/0');
    });
    test('never rises above the floor', () {
      expect(parentOf('/storage/emulated/0/a/b', '/storage/emulated/0'),
          '/storage/emulated/0/a');
    });
  });

  test('subdirsOf lists only directories, sorted, and never throws', () {
    final dir = Directory.systemTemp.createTempSync('alix-fb-');
    addTearDown(() => dir.deleteSync(recursive: true));
    Directory('${dir.path}/Zebra').createSync();
    Directory('${dir.path}/apple').createSync();
    File('${dir.path}/note.txt').writeAsStringSync('x');
    expect(subdirsOf(dir.path), ['apple', 'Zebra']);
    expect(subdirsOf('${dir.path}/missing'), isEmpty);
  });

  Widget host(void Function(String?) onResult, Map<String, List<String>> tree) {
    return MaterialApp(
      theme: alixDark(),
      home: Builder(
        builder: (context) => Scaffold(
          body: Center(
            child: ElevatedButton(
              onPressed: () async {
                final chosen = await Navigator.of(context).push<String>(
                  MaterialPageRoute(
                    builder: (_) => FolderBrowser(
                      start: '/root',
                      listDirs: (p) => tree[p] ?? const [],
                    ),
                  ),
                );
                onResult(chosen);
              },
              child: const Text('open'),
            ),
          ),
        ),
      ),
    );
  }

  testWidgets('navigates into a folder and returns its real path',
      (tester) async {
    String? picked = 'unset';
    await tester.pumpWidget(host((r) => picked = r, {
      '/root': ['Sync', 'Docs'],
      '/root/Sync': ['decks'],
    }));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();

    // At the floor: no up affordance yet.
    expect(find.text('..'), findsNothing);
    expect(find.text('Sync'), findsOneWidget);

    await tester.tap(find.text('Sync'));
    await tester.pumpAndSettle();
    expect(find.text('decks'), findsOneWidget);
    expect(find.text('..'), findsOneWidget); // now one level below the floor

    await tester.tap(find.text('Use this folder'));
    await tester.pumpAndSettle();
    expect(picked, '/root/Sync');
  });

  testWidgets('back without choosing returns null', (tester) async {
    String? picked = 'unset';
    await tester.pumpWidget(host((r) => picked = r, {
      '/root': ['Sync'],
    }));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Back')); // the AppBar back button
    await tester.pumpAndSettle();
    expect(picked, isNull);
  });
}
