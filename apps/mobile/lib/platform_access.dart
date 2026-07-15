import 'dart:io';

import 'package:flutter/services.dart';

/// Platform plumbing for choosing a shared decks folder. Behind an
/// interface because method channels do not exist under `flutter test`;
/// widget tests inject a fake.
abstract class PlatformAccess {
  /// Whether this device can point alix at a shared folder at all
  /// (Android 11+, or any desktop).
  Future<bool> supportsSharedFolders();

  /// Ensures All Files Access. On Android this may bounce through the
  /// system settings page: `false` means "not granted yet, sent the user
  /// there"; the user taps the action again after granting.
  Future<bool> ensureAllFilesAccess();

  /// The picked directory as a real filesystem path, or null on cancel.
  Future<String?> pickDirectory();
}

/// The real thing: a small platform channel into MainActivity (the
/// All-Files-Access dance plus the system folder picker; the plugin
/// ecosystem for either does not build against this project's AGP 9
/// toolchain, and four calls do not earn a dependency anyway).
class RealPlatformAccess implements PlatformAccess {
  static const _channel = MethodChannel('alix/platform');

  @override
  Future<bool> supportsSharedFolders() async {
    if (!Platform.isAndroid) {
      return true;
    }
    final sdk = await _channel.invokeMethod<int>('sdkInt') ?? 0;
    return sdk >= 30;
  }

  @override
  Future<bool> ensureAllFilesAccess() async {
    if (!Platform.isAndroid) {
      return true;
    }
    final has = await _channel.invokeMethod<bool>('hasAllFilesAccess') ?? false;
    if (has) {
      return true;
    }
    await _channel.invokeMethod<void>('requestAllFilesAccess');
    return false;
  }

  @override
  Future<String?> pickDirectory() async {
    if (Platform.isAndroid) {
      final uri = await _channel.invokeMethod<String>('pickDirectory');
      return uri == null ? null : pathFromTreeUri(uri);
    }
    // The desktop build is a dev vehicle; zenity covers it without a dep.
    try {
      final res = await Process.run('zenity', [
        '--file-selection',
        '--directory',
      ]);
      final out = (res.stdout as String).trim();
      return res.exitCode == 0 && out.isNotEmpty ? out : null;
    } on ProcessException {
      return null;
    }
  }
}

/// Maps a document-tree URI from the system picker to a real filesystem
/// path, e.g. `content://com.android.externalstorage.documents/tree/
/// primary%3Adecks` to `/storage/emulated/0/decks`. Only the device-storage
/// provider maps; anything else (cloud providers, downloads) returns null
/// and the caller tells the user to pick a folder on device storage.
String? pathFromTreeUri(String uri) {
  const treePrefix = 'content://com.android.externalstorage.documents/tree/';
  if (!uri.startsWith(treePrefix)) {
    return null;
  }
  final id = Uri.decodeComponent(uri.substring(treePrefix.length));
  final colon = id.indexOf(':');
  if (colon < 0) {
    return null;
  }
  final volume = id.substring(0, colon);
  final rest = id.substring(colon + 1);
  if (volume == 'raw') {
    return rest.isEmpty ? null : rest;
  }
  final base = volume == 'primary' ? '/storage/emulated/0' : '/storage/$volume';
  return rest.isEmpty ? base : '$base/$rest';
}
