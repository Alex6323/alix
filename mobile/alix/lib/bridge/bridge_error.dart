import 'package:flutter_rust_bridge/flutter_rust_bridge.dart'
    show AnyhowException;

/// The message of a bridged error, without the "Stack backtrace:" tail that
/// anyhow's Debug formatting appends when RUST_BACKTRACE is set (init_app
/// forces it so panic diagnostics stay captured).
String bridgeErrorText(Object error) {
  final raw = error is AnyhowException ? error.message : '$error';
  final cut = raw.indexOf('\n\nStack backtrace:');
  return cut < 0 ? raw : raw.substring(0, cut);
}
