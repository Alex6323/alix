// The tutor sheet as the review screen opens it: on the phone's geometry
// with a status bar, from a real deck through the bridge, so RustLib.init()
// is required to mount the screen.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/shared/inline_models.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/tutor_sheet.dart';

import 'support/app_fonts.dart';
import 'support/deck_fixture.dart';
import 'support/fake_server_client.dart';

const _statusBar = 37.0;
const _question = 'Why can a single `let` statement introduce more than one variable?';

void main() {
  setUpAll(() async {
    await RustLib.init();
    await loadAppFonts();
  });

  Directory tempDir(String prefix) {
    final dir = Directory.systemTemp.createTempSync(prefix);
    addTearDown(() {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    });
    return dir;
  }

  Future<void> openTutor(WidgetTester tester, {List<RemoteAsk> replies = const []}) async {
    addTearDown(tester.view.reset);
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 3.0;
    tester.view.padding = const FakeViewPadding(top: _statusBar * 3);
    final support = tempDir('alix-tutor-sheet-support-');
    final root = tempDir('alix-tutor-sheet-decks-');
    writeTestDeck('${root.path}/facts.md', '---\ntitle: Facts\n---\n## $_question\na\n');
    await savePairing(
      const ServerConfig(host: '127.0.0.1', port: 7777, token: 'tok', rootId: 'root-test0000000000000000000000'),
      support: support,
    );
    await tester.pumpWidget(MaterialApp(
      theme: alixDark(),
      home: ReviewScreen(
        deckPath: '${root.path}/facts.md',
        rootDir: root.path,
        depth: ReviewDepth.recall,
        supportDir: support,
        buildClient: (_) => FakeServerClient(versionReply: '0.6.0', getAskReplies: replies),
      ),
    ));
    await tester.pumpAndSettle();
    final reveal = find.widgetWithText(InkWell, 'Reveal');
    await tester.ensureVisible(reveal);
    await tester.tap(reveal);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('ask-glyph')));
    await tester.pumpAndSettle();
    expect(find.byType(TutorSheet), findsOneWidget, reason: 'the ask glyph opened the tutor sheet');
  }

  Finder sheetTitle() => find.descendant(
    of: find.byType(TutorSheet),
    matching: find.byWidgetPredicate(
      (widget) => widget is RichText && widget.text.toPlainText().contains('introduce more than one variable'),
    ),
  );

  testWidgets('the tutor sheet titles the card with its inline code styled, not as literal backticks', (tester) async {
    await openTutor(tester);

    final title = tester.widget<RichText>(sheetTitle());
    final plain = title.text.toPlainText().replaceAll('​', '');
    expect(plain, isNot(contains('`')), reason: 'title text: "$plain"');
    final codeSpans = <String>[];
    title.text.visitChildren((span) {
      if (span is TextSpan && span.text != null && span.style?.fontFamily == 'IBM Plex Mono') {
        codeSpans.add(span.text!.replaceAll('​', ''));
      }
      return true;
    });
    expect(codeSpans, ['let'], reason: 'the code run is the one monospace span; title text: "$plain"');
  });

  testWidgets('a tutor answer too long for the screen keeps the sheet title below the status bar', (tester) async {
    final long = List.filled(40, 'A pattern mirrors the shape of the value it destructures.').join(' ');
    await openTutor(
      tester,
      replies: [
        RemoteAsk(
          thinking: false,
          answer: long,
          units: [
            ReviewSentenceModel(
              text: long,
              runs: [InlineRunModel(text: long, bold: false, italic: false, code: false)],
            ),
          ],
        ),
      ],
    );
    await tester.enterText(find.byKey(const ValueKey('tutor-question-field')), 'how does a pattern do that?');
    await tester.tap(find.byKey(const ValueKey('tutor-send-button')));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    await tester.pumpAndSettle();
    expect(find.textContaining('A pattern mirrors', findRichText: true), findsOneWidget, reason: 'the long answer landed');

    final title = tester.getRect(sheetTitle());
    final sheet = tester.getRect(find.byType(TutorSheet));
    expect(
      title.top,
      greaterThanOrEqualTo(_statusBar),
      reason: 'title=$title sheet=$sheet status bar=$_statusBar',
    );
  });
}
