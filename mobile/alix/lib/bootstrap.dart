import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:flutter/foundation.dart' show debugPrint;
import 'package:flutter/services.dart' show rootBundle;
import 'package:path_provider/path_provider.dart';

import 'package:alix_mobile/bridge/seed_bridge.dart';
import 'package:alix_mobile/bridge/sync_bridge.dart' as sync_bridge;
import 'package:alix_mobile/server_client.dart';

/// The bundled sample decks, copied into a fresh decks dir on first run.
/// Keep in step with the `assets:` list in pubspec.yaml.
const _samples = [
  'decks/basics.md',
  'decks/sample-workspace/alix.toml',
  'decks/sample-workspace/decks/capitals.md',
  'decks/sample-workspace/decks/steps.md',
];

/// What [prepare] resolved for this launch.
class Prepared {
  const Prepared({required this.root, required this.device, this.themeId});

  /// The decks root to list.
  final String root;

  /// This install's label in the store's last-writer marker.
  final String device;

  /// The persisted color theme choice, if any; `null` resolves to the dark
  /// default via `themeById`.
  final String? themeId;
}

/// Resolves the decks root and this install's device label.
///
/// Order: `ALIX_DECKS_DIR` (the Linux desktop pointing at a real host
/// folder), else the app-private `<support>/decks`, seeded with the bundled
/// samples on first run. Existing files are never overwritten, so the file
/// names the store keys on stay stable. `support` and `env` inject the
/// platform pieces for tests.
Future<Prepared> prepare({Directory? support, String? env}) async {
  support ??= await getApplicationSupportDirectory();
  final settings = readSettings(support);
  final device = await _ensureDevice(support, settings);
  final theme = settings['theme'];
  final themeId = theme is String ? theme : null;

  env ??= Platform.environment['ALIX_DECKS_DIR'];
  if (env != null && env.isNotEmpty) {
    return Prepared(root: env, device: device, themeId: themeId);
  }
  return Prepared(root: await _appPrivate(support), device: device, themeId: themeId);
}

/// [prepare]'s result, plus a best-effort recovery of the active paired
/// root, if any: rolls back an interrupted apply and creates the directory
/// if missing, before the picker ever lists it. Never touches
/// [Prepared.root] — the phone's own root and a paired root are two
/// separate listings the picker shows side by side (`picker_screen.dart`),
/// not a redirect. A failed recovery here is swallowed; `_loadPairing`'s
/// own recovery attempt is what surfaces the error, in the first sync
/// report.
Future<Prepared> prepareWithPairing({Directory? support, String? env}) async {
  final resolvedSupport = support ?? await getApplicationSupportDirectory();
  final prepared = await prepare(support: resolvedSupport, env: env);
  final resolvedEnv = env ?? Platform.environment['ALIX_DECKS_DIR'];
  if (resolvedEnv != null && resolvedEnv.isNotEmpty) return prepared;
  final pairing = readActivePairing(resolvedSupport);
  if (pairing == null) return prepared;
  try {
    final pairedDir = sync_bridge.pairedRootDirFor(
      support: resolvedSupport.path,
      rootId: pairing.rootId,
    );
    sync_bridge.pairedRecoverFor(rootDir: pairedDir);
  } on Object catch (error) {
    debugPrint('paired recover at startup failed: $error');
  }
  return prepared;
}

/// The app-private decks dir, created and sample-seeded on first use.
Future<String> _appPrivate(Directory support) async {
  final root = Directory('${support.path}/decks');
  // The tutorial seeds ONLY into a brand-new decks dir (unlike the samples
  // below, which re-seed per file): its last card says "delete me when
  // done", so a deletion must be final. Mirrors the desktop rule in the
  // core's `tutorial::seed_new_decks_dir`; the bundled copy is pinned to
  // the canonical assets/decks/tutorial.md by a core test.
  final fresh = !await root.exists();
  await root.create(recursive: true);
  if (fresh) {
    final content = await rootBundle.loadString('assets/decks/tutorial.md');
    await File('${root.path}/tutorial.md').writeAsString(content);
    await _stampSeed('${root.path}/tutorial.md');
  }
  for (final sample in _samples) {
    final target = File('${support.path}/$sample');
    if (!await target.exists()) {
      await target.parent.create(recursive: true);
      final content = await rootBundle.loadString('assets/$sample');
      await target.writeAsString(content);
      if (sample.endsWith('.md')) await _stampSeed(target.path);
    }
  }
  return root.path;
}

// A seeded copy needs minted ids to be discovered (the bundled assets carry
// none; the tutorial asset is pinned byte-identical to the desktop's, which
// stamps at seed time too). Best-effort like the desktop seeder: a failed
// stamp leaves the file present but undiscovered until `alix deck init`.
Future<void> _stampSeed(String path) async {
  try {
    await stampSeededDeck(path);
  } on Object catch (error) {
    debugPrint('seed stamp failed for $path: $error');
  }
}

/// Copies the bundled tutorial deck into [root] (the current decks folder)
/// unless a `tutorial.md` is already there. The first-run seed only ever fills
/// a brand-new app-private dir, so this is how a folder that never got it (an
/// emptied one) can still start the tutorial from the picker's empty state.
Future<void> addTutorialDeck(String root) async {
  final file = File('$root/tutorial.md');
  if (!await file.exists()) {
    final content = await rootBundle.loadString('assets/decks/tutorial.md');
    await file.writeAsString(content);
  }
  // Stamp even when the file already existed: an unstamped copy (stranded by
  // a pre-stamping build) never lists, and stamping a stamped deck is a no-op.
  await _stampSeed(file.path);
}

File _settingsFile(Directory support) => File('${support.path}/settings.json');

/// The app's persisted choices (`settings.json` in the support dir), e.g.
/// `{"device": "phone-3f2a"}`. Unreadable or malformed settings read as
/// empty.
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

/// Every stored pairing (a `pairings` key in settings.json holding a list of
/// `ServerConfig.toJson()` maps, at most one per `rootId`). A malformed row
/// is skipped rather than failing the whole read.
List<ServerConfig> readPairings(Directory support) => _pairingsFrom(readSettings(support));

List<ServerConfig> _pairingsFrom(Map<String, dynamic> settings) {
  final raw = settings['pairings'];
  if (raw is! List) return const [];
  return raw.map(ServerConfig.fromJson).whereType<ServerConfig>().toList();
}

/// The pairing named by the `active_root` key, or null when unset or no
/// longer among [readPairings].
ServerConfig? readActivePairing(Directory support) {
  final settings = readSettings(support);
  final activeRoot = settings['active_root'];
  if (activeRoot is! String) return null;
  for (final pairing in _pairingsFrom(settings)) {
    if (pairing.rootId == activeRoot) return pairing;
  }
  return null;
}

/// Persists [config]: replaces any existing pairing for the same `rootId`,
/// or appends a new one, and makes it the active pairing.
Future<void> savePairing(ServerConfig config, {Directory? support}) async {
  support ??= await getApplicationSupportDirectory();
  final settings = readSettings(support);
  final pairings = _pairingsFrom(settings).where((p) => p.rootId != config.rootId).toList()
    ..add(config);
  settings['pairings'] = pairings.map((p) => p.toJson()).toList();
  settings['active_root'] = config.rootId;
  _writeSettings(support, settings);
}

/// Makes the pairing named [rootId] the active one.
Future<void> setActiveRoot(String rootId, {Directory? support}) async {
  support ??= await getApplicationSupportDirectory();
  final settings = readSettings(support);
  settings['active_root'] = rootId;
  _writeSettings(support, settings);
}

/// Removes the pairing named [rootId]; clears `active_root` when it pointed
/// there.
Future<void> removePairing(String rootId, {Directory? support}) async {
  support ??= await getApplicationSupportDirectory();
  final settings = readSettings(support);
  final pairings = _pairingsFrom(settings).where((p) => p.rootId != rootId).toList();
  settings['pairings'] = pairings.map((p) => p.toJson()).toList();
  if (settings['active_root'] == rootId) settings.remove('active_root');
  _writeSettings(support, settings);
}

/// The persisted color theme choice, if any (a `theme` key in
/// settings.json). Absent or malformed reads as null, never throws.
String? readTheme(Directory support) {
  final theme = readSettings(support)['theme'];
  return theme is String ? theme : null;
}

/// Persists the theme choice; `null` reverts to the default.
Future<void> setTheme(String? theme, {Directory? support}) async {
  support ??= await getApplicationSupportDirectory();
  final settings = readSettings(support);
  if (theme == null) {
    settings.remove('theme');
  } else {
    settings['theme'] = theme;
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
