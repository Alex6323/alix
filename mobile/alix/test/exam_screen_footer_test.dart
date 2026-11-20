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

  Future<void> pumpStaleAtOpen(WidgetTester tester) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: alixDark(),
      home: ExamScreen(
        deckName: 'rust.md',
        held: FakeHeldExam(changed: true),
        client: FakeServerClient(),
        onLeaveResult: () {},
        onDiscard: () async => true,
        pollInterval: const Duration(milliseconds: 10),
      ),
    ));
    await tester.pump();
    await tester.pump();
  }

  void expectNotCut(WidgetTester tester, String state, Finder message) {
    expect(message, findsOneWidget, reason: '$state: the message shows');
    final paragraph = tester.renderObject<RenderParagraph>(message);
    expect(paragraph.didExceedMaxLines, isFalse, reason: '$state: the message is not cut off');
    expect(tester.takeException(), isNull, reason: '$state: no overflow');
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

  testWidgets('on a 360-wide phone no footer label is cut beside Submit, Submit again or Discard exam', (tester) async {
    for (final (state, held, client) in [
      ('Submit', FakeHeldExam(), FakeServerClient()),
      ('Submit again', FakeHeldExam(), FakeServerClient(gradingReplies: const [GradingRefused(409)])),
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

  final staleMessage = find.textContaining('Discard it and generate a new one.');

  testWidgets('the stale view at open shows its whole message and Discard exam on a 360-wide phone', (tester) async {
    await pumpStaleAtOpen(tester);
    expectNotCut(tester, 'stale at open', staleMessage);
    expect(find.text('Discard exam'), findsOneWidget, reason: 'stale at open: Discard exam is on screen');
  });

  testWidgets('the stale view at open shows its whole message at text scale 1.5', (tester) async {
    tester.platformDispatcher.textScaleFactorTestValue = 1.5;
    addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
    await pumpStaleAtOpen(tester);
    expectNotCut(tester, 'stale at open, text scale 1.5', staleMessage);
  });

  testWidgets('the notice on Submit shows its whole message above the footer on a 360-wide phone', (tester) async {
    final held = FakeHeldExam();
    await openOnPhone(tester, held, FakeServerClient());
    await tester.enterText(_field, 'a borrow is a reference');
    held.changed = true;
    await tester.tap(find.text('Submit'));
    await tester.pump();
    await tester.pump();
    const staleText =
        'The deck has changed. This exam is stale and cannot be recorded. Discard it and generate a new one.';
    expect(find.text(staleText), findsOneWidget, reason: 'notice on Submit: the stale message is on screen');
    expectNotCut(tester, 'notice on Submit', find.text(staleText));
    for (final label in ['Back', 'Continue later', 'Discard exam']) {
      expectWhole(tester, 'notice on Submit', label);
    }
  });

  testWidgets('each desktop message on Submit shows whole at the default text size and at text scale 1.5', (tester) async {
    addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
    for (final scale in [1.0, 1.5]) {
      tester.platformDispatcher.textScaleFactorTestValue = scale;
      for (final (row, reply, message) in [
        ('needs the desktop', const GradingNoAnswer(), 'Grading needs the paired desktop.'),
        ('busy', const GradingRefused(409), 'The desktop is busy grading. Try again shortly.'),
        ('unreadable', const GradingRefused(400), 'The desktop could not read this exam. Update Alix on both devices.'),
        ('refused', const GradingRefused(503), 'The desktop refused this request (503).'),
      ]) {
        final state = '$row, text scale $scale';
        await openOnPhone(tester, FakeHeldExam(), FakeServerClient(gradingReplies: [reply]));
        await tester.enterText(_field, 'a borrow is a reference');
        await tester.tap(find.text('Submit'));
        await tester.pump();
        await tester.pump();
        expect(find.text(message), findsOneWidget, reason: '$state: the row\'s message is on screen');
        expectNotCut(tester, state, find.text(message));
        await tester.pumpWidget(const SizedBox.shrink());
      }
    }
  });
}
