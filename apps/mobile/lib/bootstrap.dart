import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:flutter/services.dart' show rootBundle;
import 'package:path_provider/path_provider.dart';

import 'package:alix_mobile/server_client.dart';

/// The bundled sample decks, copied into a fresh decks dir on first run.
/// Keep in step with the `assets:` list in pubspec.yaml.
const _samples = [
  'decks/basics.txt',
  'decks/sample-workspace/alix.toml',
  'decks/sample-workspace/capitals.txt',
  'decks/sample-workspace/steps.txt',
];

/// What [prepare] resolved for this launch.
class Prepared {
  const Prepared({
    required this.root,
    required this.device,
    this.sharedDir,
    this.staleDecksDir,
  });

  /// The decks root to list.
  final String root;

  /// This install's label in the store's last-writer marker.
  final String device;

  /// The persisted shared-folder setting, stale or not (`null` when the
  /// user never chose one).
  final String? sharedDir;

  /// Set when the settings pointed at a folder that is gone or unreadable:
  /// this launch fell back to app storage, the setting was kept so a
  /// re-grant or re-mount heals it.
  final String? staleDecksDir;
}

/// Resolves the decks root and this install's device label.
///
/// Order: `ALIX_DECKS_DIR` (the Linux desktop pointing at a real host
/// folder), then the settings' shared folder while it is still usable,
/// else the app-private `<support>/decks`, seeded with the bundled samples
/// on first run. Existing files are never overwritten, so the file names
/// the store keys on stay stable. `support` and `env` inject the platform
/// pieces for tests.
///
/// `hasStorageAccess` is the All-Files-Access query: a revoked grant does
/// not make a shared dir unlistable (Android's FUSE filters it to empty),
/// so a configured shared folder counts as usable only when this reports
/// true. `null` skips the check (desktop, tests).
Future<Prepared> prepare({
  Directory? support,
  String? env,
  Future<bool> Function()? hasStorageAccess,
}) async {
  support ??= await getApplicationSupportDirectory();
  final settings = readSettings(support);
  final device = await _ensureDevice(support, settings);
  final shared = settings['decksDir'];
  final sharedDir = shared is String && shared.isNotEmpty ? shared : null;

  env ??= Platform.environment['ALIX_DECKS_DIR'];
  if (env != null && env.isNotEmpty) {
    return Prepared(root: env, device: device, sharedDir: sharedDir);
  }

  if (sharedDir != null) {
    final granted = await hasStorageAccess?.call() ?? true;
    if (granted && _listable(sharedDir)) {
      return Prepared(root: sharedDir, device: device, sharedDir: sharedDir);
    }
    return Prepared(
      root: await _appPrivate(support),
      device: device,
      sharedDir: sharedDir,
      staleDecksDir: sharedDir,
    );
  }
  return Prepared(root: await _appPrivate(support), device: device);
}

/// True when the directory exists and can actually be listed (a revoked
/// permission surfaces as a listing error, not a missing dir).
bool _listable(String dir) {
  try {
    Directory(dir).listSync();
    return true;
  } on FileSystemException {
    return false;
  }
}

/// The app-private decks dir, created and sample-seeded on first use.
Future<String> _appPrivate(Directory support) async {
  final root = Directory('${support.path}/decks');
  // The tutorial seeds ONLY into a brand-new decks dir (unlike the samples
  // below, which re-seed per file): its last card says "delete me when
  // done", so a deletion must be final. Mirrors the desktop rule in the
  // core's `tutorial::seed_new_decks_dir`; the bundled copy is pinned to
  // the canonical assets/decks/tutorial.txt by a core test.
  final fresh = !await root.exists();
  await root.create(recursive: true);
  if (fresh) {
    final content = await rootBundle.loadString('assets/decks/tutorial.txt');
    await File('${root.path}/tutorial.txt').writeAsString(content);
  }
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

File _settingsFile(Directory support) => File('${support.path}/settings.json');

/// The app's persisted choices (`settings.json` in the support dir), e.g.
/// `{"decksDir": "/storage/emulated/0/decks", "device": "phone-3f2a"}`.
/// Unreadable or malformed settings read as empty.
Map<String, dynamic> readSettings(Directory support) {
  try {
    final decoded = jsonDecode(_settingsFile(support).readAsStringSync());
    return decoded is Map<String, dynamic> ? decoded : <String, dynamic>{};
  } on FormatException {
    return <String, dynamic>{};
  } on FileSystemException {
    return <String, dynamic>{};
  }
}

// Synchronous on purpose: the file is ~100 bytes, and sync I/O lets the
// widget-test zone (fake async, no real event-loop turns) drive the
// choose-folder flow to completion.
void _writeSettings(Directory support, Map<String, dynamic> settings) {
  support.createSync(recursive: true);
  _settingsFile(support).writeAsStringSync(jsonEncode(settings));
}

/// Persists the shared decks folder choice; `null` reverts to app storage.
Future<void> setDecksDir(String? dir, {Directory? support}) async {
  support ??= await getApplicationSupportDirectory();
  final settings = readSettings(support);
  if (dir == null) {
    settings.remove('decksDir');
  } else {
    settings['decksDir'] = dir;
  }
  _writeSettings(support, settings);
}

/// The paired desktop, if any (a `server` key in settings.json holding
/// `{host, port, token}`). Absent or malformed reads as unpaired, never
/// throws.
ServerConfig? readServer(Directory support) => ServerConfig.fromJson(readSettings(support)['server']);

/// Persists the pairing; `null` un-pairs (removes the key).
Future<void> setServer(ServerConfig? config, {Directory? support}) async {
  support ??= await getApplicationSupportDirectory();
  final settings = readSettings(support);
  if (config == null) {
    settings.remove('server');
  } else {
    settings['server'] = config.toJson();
  }
  _writeSettings(support, settings);
}

/// This install's device label (`phone-<4 hex>`), minted once into the
/// settings. It names the phone in the store's last-writer marker; it lives
/// in the support dir, never in the synced folder.
Future<String> _ensureDevice(
  Directory support,
  Map<String, dynamic> settings,
) async {
  final existing = settings['device'];
  if (existing is String && existing.isNotEmpty) {
    return existing;
  }
  final device =
      'phone-${Random().nextInt(0x10000).toRadixString(16).padLeft(4, '0')}';
  settings['device'] = device;
  _writeSettings(support, settings);
  return device;
}
