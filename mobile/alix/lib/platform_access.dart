import 'dart:io';

import 'package:flutter/services.dart';

/// Platform plumbing for choosing a shared decks folder. Behind an
/// interface because method channels do not exist under `flutter test`;
/// widget tests inject a fake.
abstract class PlatformAccess {
  /// Whether this device can point alix at a shared folder at all
  /// (Android 11+, or any desktop).
  Future<bool> supportsSharedFolders();

  /// Whether All Files Access is currently granted (a pure query, no
  /// side effect). A revoked grant does NOT make shared dirs unlistable:
  /// FUSE silently filters them to empty, so launch checks must ask this
  /// instead of probing the filesystem.
  Future<bool> hasAllFilesAccess();

  /// Ensures All Files Access. On Android this may bounce through the
  /// system settings page: `false` means "not granted yet, sent the user
  /// there"; the user taps the action again after granting.
  Future<bool> ensureAllFilesAccess();

  /// The picked directory as a real filesystem path, or null on cancel.
  Future<String?> pickDirectory();

  /// The installed package's version (`X.Y.Z+N`), read from the platform so
  /// About can never drift from the build; null where no package exists
  /// (the desktop dev build).
  Future<String?> appVersion();
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
  Future<bool> hasAllFilesAccess() async {
    if (!Platform.isAndroid) {
      return true;
    }
    return await _channel.invokeMethod<bool>('hasAllFilesAccess') ?? false;
  }

  @override
  Future<bool> ensureAllFilesAccess() async {
    if (await hasAllFilesAccess()) {
      return true;
    }
    await _channel.invokeMethod<void>('requestAllFilesAccess');
    return false;
  }

  @override
  Future<String?> appVersion() async {
    if (!Platform.isAndroid) {
      return null;
    }
    return _channel.invokeMethod<String>('appVersion');
  }

  @override
  Future<String?> pickDirectory() async {
    // Android chooses via the in-app FolderBrowser (the system SAF picker's
    // DocumentsUI crashes on some devices, and full-filesystem access makes it
    // unnecessary). This path stays for the desktop dev vehicle, where zenity
    // covers it without a dep.
    if (Platform.isAndroid) return null;
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
