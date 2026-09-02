import 'package:alix_mobile/bridge/bridge_error.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge.dart'
    show AnyhowException;
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('a backtrace tail is stripped from a bridged error message', () {
    final e = AnyhowException(
      'milestone 2 reviews a facts deck, not a trace\n\n'
      'Stack backtrace:\n   0: <unknown>\n   1: frb_pde_ffi_dispatcher_sync',
    );
    expect(
      bridgeErrorText(e),
      'milestone 2 reviews a facts deck, not a trace',
    );
  });

  test('a clean message and a non-anyhow error pass through unchanged', () {
    expect(bridgeErrorText(AnyhowException('deck is empty')), 'deck is empty');
    expect(bridgeErrorText(StateError('bad')), 'Bad state: bad');
  });
}
