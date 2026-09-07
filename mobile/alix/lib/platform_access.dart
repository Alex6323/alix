import 'dart:io';

import 'package:flutter/services.dart';

/// Platform plumbing behind an interface because method channels do not
/// exist under `flutter test`; widget tests inject a fake.
abstract class PlatformAccess {
  /// The picked directory as a real filesystem path, or null on cancel.
  Future<String?> pickDirectory();

  /// The installed package's version (`X.Y.Z+N`), read from the platform so
  /// About can never drift from the build; null where no package exists
  /// (the desktop dev build).
  Future<String?> appVersion();
}

/// The real thing: a small platform channel into MainActivity (the system
/// folder picker; the plugin ecosystem for it does not build against this
/// project's AGP 9 toolchain, and two calls do not earn a dependency anyway).
class RealPlatformAccess implements PlatformAccess {
  static const _channel = MethodChannel('alix/platform');

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
