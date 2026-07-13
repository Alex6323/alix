import 'package:flutter/material.dart';

import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/api/listing.dart';
import 'package:alix_mobile/src/rust/api/review.dart';

/// One list screen serving both levels: the decks root, and (with [dir]) a
/// drilled-into workspace or deck folder.
class PickerScreen extends StatefulWidget {
  const PickerScreen({super.key, required this.root, this.dir, this.title});

  final String root;
  final String? dir;
  final String? title;

  @override
  State<PickerScreen> createState() => _PickerScreenState();
}

class _PickerScreenState extends State<PickerScreen> {
  late List<DeckEntry> _entries;

  @override
  void initState() {
    super.initState();
    _load();
  }

  void _load() {
    final dir = widget.dir;
    _entries = dir == null
        ? listRoot(root: widget.root)
        : listMembers(root: widget.root, dir: dir);
  }

  Future<void> _openDeck(DeckEntry entry) async {
    final depth = await _pickDepth(context);
    if (depth == null || !mounted) return;
    await Navigator.of(context).push(
      MaterialPageRoute(
        builder: (_) => ReviewScreen(
          deckPath: entry.path,
          rootDir: widget.root,
          depth: depth,
        ),
      ),
    );
    // Progress changed while reviewing; refresh the due dots.
    setState(_load);
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: Text(widget.title ?? 'alix')),
      body: _entries.isEmpty
          ? const Center(child: Text('no decks here'))
          : ListView(
              children: [
                for (final entry in _entries)
                  ListTile(
                    leading: Icon(
                      entry.isWorkspace ? Icons.folder_outlined : Icons.description_outlined,
                    ),
                    title: Text(
                      entry.title,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                    trailing: entry.due
                        ? Icon(
                            Icons.circle,
                            size: 10,
                            color: Theme.of(context).colorScheme.primary,
                          )
                        : null,
                    onTap: () {
                      if (entry.isWorkspace) {
                        Navigator.of(context).push(
                          MaterialPageRoute(
                            builder: (_) => PickerScreen(
                              root: widget.root,
                              dir: entry.path,
                              title: entry.title,
                            ),
                          ),
                        );
                      } else {
                        _openDeck(entry);
                      }
                    },
                  ),
              ],
            ),
    );
  }
}

/// The session depth pick, defaulting to Recall.
Future<Depth?> _pickDepth(BuildContext context) {
  return showModalBottomSheet<Depth>(
    context: context,
    builder: (context) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          for (final (depth, label, hint) in [
            (Depth.recognize, 'Recognize', 'pick the answer out of four'),
            (Depth.recall, 'Recall', 'the everyday review'),
            (Depth.reconstruct, 'Reconstruct', 'type or rebuild the answer'),
          ])
            ListTile(
              title: Text(label),
              subtitle: Text(hint),
              onTap: () => Navigator.of(context).pop(depth),
            ),
        ],
      ),
    ),
  );
}
