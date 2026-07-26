import 'dart:io';

void writeTestDeck(String path, String contents, {String? id}) {
  final filename = File(path).uri.pathSegments.last;
  final deckId =
      id ?? filename.replaceAll(RegExp(r'[^A-Za-z0-9]'), '').toLowerCase();
  final initialized = contents.startsWith('---\n')
      ? contents.replaceFirst('---\n', '---\nalix-id: "$deckId"\n')
      : '---\nalix-id: "$deckId"\n---\n$contents';
  File(path).writeAsStringSync(initialized);
}
