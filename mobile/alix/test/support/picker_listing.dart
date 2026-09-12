import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/picker/picker_view.dart';

/// Waits for every picker on screen to finish its first listing, or for
/// [until] to hold when a test needs a later relisting's answer instead.
///
/// The listing is an async bridge call answered from a native thread; the
/// fake test zone never delivers that answer on a pump, so this polls inside
/// `runAsync`, bounded so a stalled core fails the test instead of hanging
/// it. A test that builds a picker without a `supportDir` registers
/// [answerPathProvider] in `setUp` first.
Future<void> settlePicker(
  WidgetTester tester, {
  bool Function()? until,
}) async {
  await tester.runAsync(() async {
    final deadline = DateTime.now().add(const Duration(seconds: 5));
    while (DateTime.now().isBefore(deadline)) {
      await tester.pump(const Duration(milliseconds: 50));
      if (until?.call() ?? _everyPickerListed()) return;
      await Future<void>.delayed(const Duration(milliseconds: 5));
    }
    fail('a picker never finished its listing');
  });
  await tester.pumpAndSettle();
  await _drainImageLoads(tester);
}

/// A row's raster icon is a real file read under `runAsync`; left in flight
/// past the test's end it fails against the deleted temp root and lands in
/// the next test.
Future<void> _drainImageLoads(WidgetTester tester) async {
  final images = find.byType(Image, skipOffstage: false).evaluate().toList();
  if (images.isEmpty) return;
  await tester.runAsync(() async {
    for (final element in images) {
      await precacheImage((element.widget as Image).image, element);
    }
  });
  await tester.pump();
}

bool _everyPickerListed() {
  final pickers = find.byType(PickerView, skipOffstage: false);
  final loading = find.byWidgetPredicate(
    (widget) => widget is PickerView && widget.isLoading,
    skipOffstage: false,
  );
  return pickers.evaluate().isNotEmpty && loading.evaluate().isEmpty;
}

const _pathProvider = MethodChannel('plugins.flutter.io/path_provider');

/// Answers the screen's `path_provider` call with a fresh temp dir; real
/// time inside `runAsync` lets that call reach the platform, which the host
/// has no plugin for.
void answerPathProvider() {
  final support = Directory.systemTemp.createTempSync('alix-support-');
  addTearDown(() => support.deleteSync(recursive: true));
  TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .setMockMethodCallHandler(_pathProvider, (call) async => support.path);
}
