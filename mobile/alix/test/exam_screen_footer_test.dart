import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/exam_screen.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/theme.dart';

import 'support/fake_held_exam.dart';
import 'support/fake_server_client.dart';

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
  await sans.load();
  await mono.load();
}

final _field = find.byKey(const ValueKey('exam-answer-field'));

void main() {
  setUpAll(_loadFonts);

  Future<void> openOnPhone(WidgetTester tester, FakeHeldExam held, ServerClient client) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: alixDark(),
      home: ExamScreen(
        deckName: 'rust.md',
        held: held,
        client: client,
        onLeaveResult: () {},
        onDiscard: () async => true,
        pollInterval: const Duration(milliseconds: 10),
      ),
    ));
    await tester.pump();
    await tester.enterText(_field, 'ownership moves values');
    await tester.tap(find.text('Next'));
    await tester.pump();
  }

  void expectWhole(WidgetTester tester, String state, String label) {
    final paragraph = tester.renderObject<RenderParagraph>(find.text(label));
    final whole = paragraph.getMaxIntrinsicWidth(double.infinity);
    expect(
      (paragraph.didExceedMaxLines, paragraph.size.width >= whole - 0.01),
      (false, true),
      reason: '$state: "$label" is laid out $whole wide and gets ${paragraph.size.width}',
    );
  }

  testWidgets('on a 360-wide phone no footer label is cut beside Submit, submit again or Discard exam', (tester) async {
    for (final (state, held, client) in [
      ('Submit', FakeHeldExam(), FakeServerClient()),
      ('submit again', FakeHeldExam(), FakeServerClient(gradingReplies: const [GradingRefused(409)])),
      ('Discard exam', FakeHeldExam(), FakeServerClient()),
    ]) {
      await openOnPhone(tester, held, client);
      await tester.enterText(_field, 'a borrow is a reference');
      if (state != 'Submit') {
        held.changed = state == 'Discard exam';
        await tester.tap(find.text('Submit'));
        await tester.pump();
        await tester.pump();
      }
      expect(find.widgetWithText(FilledButton, state), findsOneWidget, reason: '$state: the filled button reads $state');
      expect(find.text(state), findsOneWidget, reason: '$state: one control reads $state');
      for (final label in ['Back', 'Continue later', state]) {
        expectWhole(tester, state, label);
      }
      expect(tester.takeException(), isNull, reason: '$state: the footer lays out without overflow');
      await tester.pumpWidget(const SizedBox.shrink());
    }
  });
}
