import 'dart:io';

void writeTestDeck(String path, String contents, {String? id}) {
  final filename = File(path).uri.pathSegments.last;
  final deckId =
      id ?? filename.replaceAll(RegExp(r'[^A-Za-z0-9]'), '').toLowerCase();
  final initialized = contents.startsWith('---\n')
      ? contents.replaceFirst('---\n', '---\nformat-version: 1\nid: "deck-$deckId"\n')
      : '---\nformat-version: 1\nid: "deck-$deckId"\n---\n$contents';
  File(path).writeAsStringSync(initialized);
}

/// Gives the workspace member deck at `deckPath` (in `<workspace>/decks/`)
/// one source bound to its leading topic, so the core lists it with an exam.
void writeTestSources(String deckPath) {
  final deckId = RegExp(r'^id: "(deck-[^"]+)"$', multiLine: true)
      .firstMatch(File(deckPath).readAsStringSync())!
      .group(1)!;
  final workspace = File(deckPath).parent.parent.path;
  final dir = Directory('$workspace/sources/$deckId')..createSync(recursive: true);
  File('${dir.path}/manifest.toml').writeAsStringSync(
    'version = 1\n\n[sources]\n'
    'source-${'0' * 52} = { path = "live:source.txt", bound-to = ["$deckId"] }\n',
  );
}
