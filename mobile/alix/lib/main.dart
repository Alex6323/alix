import 'dart:io';

import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/bridge/sync_bridge.dart' as sync_bridge;
import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/platform_access.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  final access = RealPlatformAccess();
  final prepared = await _prepareWithPairing();
  runApp(AlixApp(prepared: prepared, access: access));
}

/// `prepare()`'s root, redirected to the active paired root when one is
/// set and no dev override (`ALIX_DECKS_DIR`) is in play: once paired, the
/// phone opens on the desktop's content rather than its own bundled
/// samples. Ensures the paired directory exists and rolls back an
/// interrupted apply before the picker ever lists it.
Future<Prepared> _prepareWithPairing() async {
  final prepared = await prepare();
  if ((Platform.environment['ALIX_DECKS_DIR'] ?? '').isNotEmpty) {
    return prepared;
  }
  final support = await getApplicationSupportDirectory();
  final pairing = readActivePairing(support);
  if (pairing == null) return prepared;
  final pairedDir = sync_bridge.pairedRootDirFor(
    support: support.path,
    rootId: pairing.rootId,
  );
  sync_bridge.pairedRecoverFor(rootDir: pairedDir);
  return Prepared(
    root: pairedDir,
    device: prepared.device,
    themeId: prepared.themeId,
  );
}

/// The app shell: holds the resolved decks root and the live theme choice.
class AlixApp extends StatefulWidget {
  const AlixApp({
    super.key,
    required this.prepared,
    this.access,
    this.persistTheme,
  });

  final Prepared prepared;

  /// Injected in widget tests; the real platform plumbing otherwise.
  final PlatformAccess? access;

  /// Persists the theme choice; tests inject one bound to their temp
  /// support dir.
  final Future<void> Function(String?)? persistTheme;

  @override
  State<AlixApp> createState() => _AlixAppState();
}

class _AlixAppState extends State<AlixApp> {
  late String? _themeId = widget.prepared.themeId;

  PlatformAccess get _access => widget.access ?? RealPlatformAccess();

  /// Persists the theme choice, then re-themes the whole app live: no
  /// restart needed.
  Future<void> _setTheme(String? id) async {
    await (widget.persistTheme ?? setTheme)(id);
    setState(() => _themeId = id);
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'alix',
      // A named gallery choice (the web app's Theme picker), not a
      // light/dark mode toggle: ONE resolved theme; themeById falls back to
      // the dark default for an unknown or absent saved id.
      theme: themeById(_themeId),
      home: PickerScreen(
        root: widget.prepared.root,
        device: widget.prepared.device,
        access: _access,
        currentThemeId: _themeId,
        onSetTheme: _setTheme,
      ),
    );
  }
}
