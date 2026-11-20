import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/picker/picker_widgets.dart';
import 'package:alix_mobile/theme.dart';

Future<void> _loadFonts() async {
  Future<ByteData> bytes(String path) async =>
      (await File(path).readAsBytes()).buffer.asByteData();
  final sans = FontLoader('IBM Plex Sans');
  for (final weight in const ['400', '500', '600', '700']) {
    sans.addFont(bytes('assets/fonts/ibm-plex-sans-$weight.ttf'));
  }
  final mono = FontLoader('IBM Plex Mono');
  for (final weight in const ['400', '600']) {
    mono.addFont(bytes('assets/fonts/ibm-plex-mono-$weight.ttf'));
  }
  final icons = FontLoader('MaterialIcons')
    ..addFont(bytes('build/unit_test_assets/fonts/MaterialIcons-Regular.otf'));
  await sans.load();
  await mono.load();
  await icons.load();
}

const _phone = Size(360, 760);
const _shot = ValueKey('shot');

final _longFailure = List.filled(
  10,
  "'claude' timed out after 600s waiting for the backend",
).join(' ');

const _desktopFailure = "'claude' timed out after 600s";

Future<void> _pumpSheet(WidgetTester tester, double scale, {String failure = ''}) async {
  tester.view.physicalSize = const Size(1080, 2280);
  tester.view.devicePixelRatio = 3;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    RepaintBoundary(
      key: _shot,
      child: MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: alixDark(),
        builder: (context, child) => MediaQuery(
          data: MediaQuery.of(context).copyWith(textScaler: TextScaler.linear(scale)),
          child: child!,
        ),
        home: Scaffold(
          body: PickerDepthSheet(
            canRecognize: true,
            onChoose: (_) {},
            onWalk: (_) {},
            exam: PickerExamOffer(
              locked: false,
              serverLive: true,
              due: false,
              filterable: false,
              failure: failure,
              onStart: (_) {},
            ),
          ),
        ),
      ),
    ),
  );
  await tester.ensureVisible(find.text('Generate exam'));
  await tester.pump();
}

Future<void> _render(WidgetTester tester, String name) async {
  final dir = Platform.environment['ALIX_RENDER_DIR'];
  if (dir == null) return;
  await tester.pump(const Duration(milliseconds: 300));
  await tester.runAsync(() async {
    final boundary = tester.renderObject<RenderRepaintBoundary>(find.byKey(_shot));
    final image = await boundary.toImage(pixelRatio: 3);
    final png = await image.toByteData(format: ui.ImageByteFormat.png);
    await File('$dir/exam-failure-$name.png').writeAsBytes(png!.buffer.asUint8List());
  });
}

RenderParagraph _paragraph(WidgetTester tester, Finder text) =>
    tester.renderObject<RenderParagraph>(text);

void main() {
  setUpAll(_loadFonts);

  for (final scale in [1.0, 1.5]) {
    testWidgets('a long failure keeps the exam row to one cut line at text scale $scale', (tester) async {
      await _pumpSheet(tester, scale, failure: _longFailure);

      final hint = _paragraph(tester, find.text('failed: $_longFailure'));
      expect(hint.maxLines, 1, reason: 'scale $scale, step hint: maxLines ${hint.maxLines}');
      expect(hint.didExceedMaxLines, isTrue, reason: 'scale $scale, step hint: the long text is cut');

      final label = _paragraph(tester, find.text('Generate exam'));
      expect(label.didExceedMaxLines, isFalse, reason: 'scale $scale, step label: not cut');

      final screen = Offset.zero & _phone;
      for (final (name, finder) in [
        ('Why it failed button', find.byTooltip('Why it failed')),
        ('chevron', find.descendant(of: find.widgetWithText(InkWell, 'Generate exam'), matching: find.byIcon(Icons.chevron_right))),
      ]) {
        expect(finder, findsOneWidget, reason: 'scale $scale, step $name: present');
        final rect = tester.getRect(finder);
        expect(screen.contains(rect.topLeft) && screen.contains(rect.bottomRight), isTrue,
            reason: 'scale $scale, step $name: $rect on the $_phone screen');
      }
      expect(tester.takeException(), isNull, reason: 'scale $scale, step layout: no exception');
    });
  }

  testWidgets('the failure dialog at text scale 1.5 shows the whole text and throws nothing', (tester) async {
    await _pumpSheet(tester, 1.5, failure: _longFailure);

    await tester.tap(find.byTooltip('Why it failed'));
    await tester.pumpAndSettle();

    expect(find.text('The exam could not be generated'), findsOneWidget, reason: 'scale 1.5, step dialog: title');
    expect(
      find.descendant(of: find.byType(AlertDialog), matching: find.widgetWithText(SelectableText, _longFailure)),
      findsOneWidget,
      reason: 'scale 1.5, step dialog: the whole text in a SelectableText',
    );
    expect(tester.takeException(), isNull, reason: 'scale 1.5, step dialog: no exception');
  });

  for (final scale in [1.0, 1.5]) {
    testWidgets('renders the exam row with a failure at text scale $scale when ALIX_RENDER_DIR is set', (tester) async {
      await _pumpSheet(tester, scale, failure: _desktopFailure);
      await _render(tester, 'row-scale-${scale.toString().replaceAll('.', '_')}');
    });
  }

  testWidgets('renders the failure dialog at text scale 1.0 when ALIX_RENDER_DIR is set', (tester) async {
    await _pumpSheet(tester, 1.0, failure: _desktopFailure);
    await tester.tap(find.byTooltip('Why it failed'));
    await tester.pumpAndSettle();
    await _render(tester, 'dialog-scale-1_0');
  });
}
