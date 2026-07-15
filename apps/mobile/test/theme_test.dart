// The ThemeData port carries the web tokens (assets/web/theme.css) exactly:
// the brand primary, the palette surfaces, and the AlixTokens extension.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/theme.dart';

void main() {
  Future<(ThemeData, AlixTokens)> pumped(WidgetTester tester, ThemeMode mode) async {
    late BuildContext ctx;
    await tester.pumpWidget(MaterialApp(
      theme: alixLight(),
      darkTheme: alixDark(),
      themeMode: mode,
      home: Builder(
        builder: (c) {
          ctx = c;
          return const SizedBox();
        },
      ),
    ));
    final theme = Theme.of(ctx);
    return (theme, theme.extension<AlixTokens>()!);
  }

  testWidgets('the dark theme carries the web tokens', (tester) async {
    final (theme, tokens) = await pumped(tester, ThemeMode.dark);
    expect(theme.colorScheme.primary, const Color(0xFFF0883C),
        reason: 'the brand chip never reskins');
    expect(theme.colorScheme.onPrimary, const Color(0xFF1A1206));
    expect(theme.colorScheme.surface, const Color(0xFF0F1016));
    expect(theme.colorScheme.onSurface, const Color(0xFFEEF0F5));
    expect(tokens.good, const Color(0xFF86C986));
    expect(tokens.warn, const Color(0xFFE6B45C));
    expect(tokens.again, const Color(0xFFE88F8F));
    expect(tokens.bolt, const Color(0xFF5FD7E0));
    expect(theme.textTheme.bodyMedium?.fontFamily, 'IBM Plex Sans');
    expect(theme.appBarTheme.titleTextStyle?.fontFamily, 'IBM Plex Mono',
        reason: 'the terminal-lineage header');
  });

  testWidgets('the light theme swaps the palette, not the brand',
      (tester) async {
    final (theme, tokens) = await pumped(tester, ThemeMode.light);
    expect(theme.colorScheme.primary, const Color(0xFFF0883C));
    expect(theme.colorScheme.surface, const Color(0xFFF4F4FA));
    expect(tokens.good, const Color(0xFF138A5B));
    expect(tokens.again, const Color(0xFFD23B34));
    expect(tokens.bolt, const Color(0xFF0E7C86));
  });
}
