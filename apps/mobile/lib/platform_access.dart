import 'dart:io';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/services.dart';

/// Platform plumbing for choosing a shared decks folder. Behind an
/// interface because plugin and method channels do not exist under
/// `flutter test`; widget tests inject a fake.
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

/// The real thing: a small platform channel into MainActivity for the
/// All-Files-Access dance (three calls do not earn a plugin dependency),
/// file_picker for the directory picker.
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
  Future<String?> pickDirectory() => FilePicker.getDirectoryPath();
}
