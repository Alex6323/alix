import 'dart:io';

import 'package:flutter/services.dart';

/// Platform plumbing behind an interface because method channels do not
/// exist under `flutter test`; widget tests inject a fake.
abstract class PlatformAccess {
  /// The installed package's version (`X.Y.Z+N`), read from the platform so
  /// About can never drift from the build; null where no package exists
  /// (the desktop dev build).
  Future<String?> appVersion();
}

/// The real thing: a small platform channel into MainActivity (one call does
/// not earn a plugin dependency).
class RealPlatformAccess implements PlatformAccess {
  static const _channel = MethodChannel('alix/platform');

  @override
  Future<String?> appVersion() async {
    if (!Platform.isAndroid) {
      return null;
    }
    return _channel.invokeMethod<String>('appVersion');
  }
}
