import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

void main() {
  test('generated Rust imports stay behind bridge and bootstrap boundaries', () {
    final violations = <String>[];
    for (final source in _dartSources()) {
      final path = _normalizedPath(source);
      if (_isGenerated(path) ||
          path.startsWith('lib/bridge/') ||
          path == 'lib/main.dart') {
        continue;
      }
      for (final (index, line) in source.readAsLinesSync().indexed) {
        if (line.trimLeft().startsWith('import ') &&
            line.contains('src/rust/')) {
          violations.add('$path:${index + 1} ${line.trim()}');
        }
      }
    }

    expect(
      violations,
      isEmpty,
      reason:
          'feature controllers, views, and widgets must depend on bridge-neutral '
          'models and ports',
    );
  });

  test('no mobile surface renders a card section', () {
    // The section rides every card on the wire (CardView.sectionContext), and
    // the ruling is that it is shown ON DEMAND, never by default. Mobile has
    // no keyboard, so the adult client's `c` press does not transfer and no
    // affordance has been ruled for it yet. Until one is, a mobile surface
    // reading the field could only be showing it unasked.
    //
    // Delete this test in the same change that adds the ruled affordance;
    // do not weaken it to let one reader through.
    final readers = <String>[];
    for (final source in _dartSources()) {
      final path = _normalizedPath(source);
      if (_isGenerated(path)) continue;
      for (final (index, line) in source.readAsLinesSync().indexed) {
        if (line.contains('sectionContext')) {
          readers.add('$path:${index + 1} ${line.trim()}');
        }
      }
    }

    expect(
      readers,
      isEmpty,
      reason:
          'a mobile surface reads sectionContext, but no on-demand affordance '
          'has been ruled for mobile yet',
    );
  });

  test('source metrics exclude generated Rust sources', () {
    final allSources = _dartSources();
    final generated = allSources.where(
      (source) => _isGenerated(_normalizedPath(source)),
    );
    final metrics = _hotspotLineCounts(allSources);

    expect(generated, isNotEmpty, reason: 'the exclusion must not be vacuous');
    expect(metrics.keys.where(_isGenerated), isEmpty);
    expect(metrics, hasLength(allSources.length - generated.length));
    expect(metrics, contains('lib/review_screen.dart'));
    expect(metrics, contains('lib/picker_screen.dart'));
    expect(metrics, contains('lib/walk_screen.dart'));
  });
}

List<File> _dartSources() {
  return Directory('lib')
      .listSync(recursive: true)
      .whereType<File>()
      .where((source) => source.path.endsWith('.dart'))
      .toList()
    ..sort((left, right) => left.path.compareTo(right.path));
}

Map<String, int> _hotspotLineCounts(Iterable<File> sources) {
  return {
    for (final source in sources)
      if (!_isGenerated(_normalizedPath(source)))
        _normalizedPath(source): source.readAsLinesSync().length,
  };
}

String _normalizedPath(File source) => source.path.replaceAll('\\', '/');

bool _isGenerated(String path) => path.startsWith('lib/src/rust/');
