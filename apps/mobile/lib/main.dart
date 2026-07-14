import 'package:flutter/material.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/platform_access.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  final prepared = await prepare();
  runApp(AlixApp(prepared: prepared));
}

/// The app shell: holds the resolved decks root and swaps it live when the
/// user points alix at a different folder.
class AlixApp extends StatefulWidget {
  const AlixApp({
    super.key,
    required this.prepared,
    this.access,
    this.reprepare,
    this.persistDecksDir,
  });

  final Prepared prepared;

  /// Injected in widget tests; the real platform plumbing otherwise.
  final PlatformAccess? access;

  /// Re-resolves the root after a folder change; tests inject one bound to
  /// their temp support dir.
  final Future<Prepared> Function()? reprepare;

  /// Persists the folder choice; tests inject one bound to their temp
  /// support dir.
  final Future<void> Function(String?)? persistDecksDir;

  @override
  State<AlixApp> createState() => _AlixAppState();
}

class _AlixAppState extends State<AlixApp> {
  late Prepared _prepared = widget.prepared;

  Future<void> _setDecksDir(String? dir) async {
    await (widget.persistDecksDir ?? setDecksDir)(dir);
    final fresh = await (widget.reprepare ?? prepare)();
    setState(() => _prepared = fresh);
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'alix',
      theme: ThemeData(colorSchemeSeed: Colors.indigo),
      home: PickerScreen(
        // Remount the whole picker tree when the root swaps.
        key: ValueKey(_prepared.root),
        root: _prepared.root,
        device: _prepared.device,
        sharedDir: _prepared.sharedDir,
        staleDecksDir: _prepared.staleDecksDir,
        access: widget.access ?? RealPlatformAccess(),
        onSetDecksDir: _setDecksDir,
      ),
    );
  }
}
