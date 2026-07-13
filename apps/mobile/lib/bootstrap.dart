import 'dart:io';

import 'package:flutter/services.dart' show rootBundle;
import 'package:path_provider/path_provider.dart';

/// Resolves the app's private data directory and makes sure the bundled
/// sample deck exists there (copied from assets on first run, left alone
/// after so the file the store keys on stays stable). Returns the deck path
/// and the store directory to hand to the Rust core.
///
/// The support dir, not the documents dir: on desktop the documents dir is
/// the user's real ~/Documents, which app data must not pollute. Support is
/// app-private everywhere (the Android files dir; `~/.local/share/<app-id>`
/// on Linux).
Future<({String deckPath, String storeDir})> prepare() async {
  final dir = await getApplicationSupportDirectory();
  final deck = File('${dir.path}/sample.txt');
  if (!await deck.exists()) {
    final content = await rootBundle.loadString('assets/sample.txt');
    await deck.writeAsString(content);
  }
  return (deckPath: deck.path, storeDir: dir.path);
}
