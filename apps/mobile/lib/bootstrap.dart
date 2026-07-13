import 'dart:io';

import 'package:flutter/services.dart' show rootBundle;
import 'package:path_provider/path_provider.dart';

/// The bundled sample decks, copied into a fresh decks dir on first run.
/// Keep in step with the `assets:` list in pubspec.yaml.
const _samples = [
  'decks/basics.txt',
  'decks/sample-workspace/alix.toml',
  'decks/sample-workspace/capitals.txt',
  'decks/sample-workspace/steps.txt',
];

/// Resolves the decks root: `ALIX_DECKS_DIR` when set (the Linux desktop
/// pointing at a real host folder), else the app-private `<support>/decks`,
/// seeded with the bundled samples on first run. Existing files are never
/// overwritten, so the file names the store keys on stay stable.
Future<String> prepare() async {
  final env = Platform.environment['ALIX_DECKS_DIR'];
  if (env != null && env.isNotEmpty) {
    return env;
  }
  final support = await getApplicationSupportDirectory();
  final root = Directory('${support.path}/decks');
  await root.create(recursive: true);
  for (final sample in _samples) {
    final target = File('${support.path}/$sample');
    if (!await target.exists()) {
      await target.parent.create(recursive: true);
      final content = await rootBundle.loadString('assets/$sample');
      await target.writeAsString(content);
    }
  }
  return root.path;
}
