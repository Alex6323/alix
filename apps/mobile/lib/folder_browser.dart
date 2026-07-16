import 'dart:io';

import 'package:flutter/material.dart';

import 'theme.dart';

/// The immediate subdirectories of [path] (names only), sorted
/// case-insensitively. Empty on any listing error, so an unreadable folder
/// (e.g. `Android/data`, still restricted under All Files Access) degrades to
/// "no subfolders" instead of throwing.
List<String> subdirsOf(String path) {
  try {
    final names = [
      for (final entry in Directory(path).listSync(followLinks: false))
        if (entry is Directory) entry.path.split('/').last,
    ];
    names.sort((a, b) => a.toLowerCase().compareTo(b.toLowerCase()));
    return names;
  } on FileSystemException {
    return const [];
  }
}

/// The parent of [current], or null once [current] is the [floor]: the browser
/// opens at the floor and never rises above it.
String? parentOf(String current, String floor) {
  if (current == floor) return null;
  final cut = current.lastIndexOf('/');
  if (cut <= 0) return null;
  final parent = current.substring(0, cut);
  return parent.length >= floor.length ? parent : null;
}

/// An in-app folder chooser over the real filesystem (All Files Access on
/// Android). It replaces the system SAF picker, whose DocumentsUI crashes on
/// some devices with a `CACHE_CONTENT` SecurityException; alix holds full
/// filesystem access, so it never needed SAF to name a folder. Pushed as a
/// route; pops the chosen absolute path, or null on back/cancel.
class FolderBrowser extends StatefulWidget {
  const FolderBrowser({
    super.key,
    required this.start,
    this.floor,
    this.listDirs = subdirsOf,
  });

  /// Where the browser opens (the user's primary storage on Android).
  final String start;

  /// Never navigate above this; defaults to [start].
  final String? floor;

  /// The directory lister, injected so widget tests need no real filesystem.
  final List<String> Function(String) listDirs;

  @override
  State<FolderBrowser> createState() => _FolderBrowserState();
}

class _FolderBrowserState extends State<FolderBrowser> {
  late String _current = widget.start;
  String get _floor => widget.floor ?? widget.start;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    final dirs = widget.listDirs(_current);
    final parent = parentOf(_current, _floor);
    return Scaffold(
      appBar: AppBar(title: const Text('Decks folder')),
      body: Column(
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 12),
            child: Text(
              _current,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: theme.textTheme.bodySmall
                  ?.copyWith(color: tokens.dim, fontFamily: 'monospace'),
            ),
          ),
          Divider(height: 1, color: tokens.line),
          Expanded(
            child: ListView(
              children: [
                if (parent != null)
                  ListTile(
                    leading: Icon(Icons.arrow_upward, color: tokens.faint),
                    title: Text('..',
                        style: TextStyle(
                            color: tokens.dim, fontFamily: 'monospace')),
                    onTap: () => setState(() => _current = parent),
                  ),
                for (final name in dirs)
                  ListTile(
                    leading: Icon(Icons.folder_outlined, color: tokens.bolt),
                    title: Text(name),
                    onTap: () => setState(() => _current = '$_current/$name'),
                  ),
                if (dirs.isEmpty)
                  Padding(
                    padding: const EdgeInsets.all(24),
                    child: Text(
                      'No subfolders here. Choose this folder, or step back.',
                      style: theme.textTheme.bodySmall
                          ?.copyWith(color: tokens.faint),
                    ),
                  ),
              ],
            ),
          ),
          SafeArea(
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: FilledButton(
                onPressed: () => Navigator.of(context).pop(_current),
                child: const Text('Use this folder'),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
