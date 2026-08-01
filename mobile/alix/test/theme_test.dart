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

  // themeFromVars's fall-back cascade (--text/--faint/--accent-ink inherit
  // DARK's explicit CSS values when a theme block omits them - the real
  // cascade binds `:root, [data-theme="dark"]` to set all 4 extras
  // explicitly, and `:root` applies regardless of the active `data-theme`,
  // so an omitting theme's --text resolves to dark's #c9cdd8, never its
  // own --ink; verified empirically in Chromium against the real
  // assets/web/theme.css - see ThemeVars's doc comment in theme.dart).
  // None of the 4 shipped themes (dark/light/nord/solarized-light)
  // actually omit any of the 3 in assets/web/theme.css today, so this is
  // exercised here with a synthetic ThemeVars, not via the CSS drift guard
  // (test/theme_gallery_test.dart).
  const probe = ThemeVars(
    surface: Color(0xFF112233),
    ink: Color(0xFFAABBCC),
    dim: Color(0xFF445566),
    line: Color(0xFF778899),
    bolt: Color(0xFF001122),
    boltHi: Color(0xFF334455),
    good: Color(0xFF00FF00),
    warn: Color(0xFFFFFF00),
    again: Color(0xFFFF0000),
    noteBorder: Color(0xFF123456),
    noteInk: Color(0xFF654321),
    // text / faint / accentInk intentionally omitted.
  );

  test('an omitted extra falls back to dark\'s explicit CSS value', () {
    final data = themeFromVars(probe, Brightness.dark);
    final tokens = data.extension<AlixTokens>()!;
    expect(tokens.text, const Color(0xFFC9CDD8),
        reason: '--text falls back to dark\'s explicit --text, not --ink');
    expect(tokens.faint, const Color(0xFF6B7085),
        reason: '--faint falls back to dark\'s explicit --faint, not --dim');
    expect(data.colorScheme.onSecondary, const Color(0xFF08131A),
        reason:
            '--accent-ink falls back to dark\'s explicit --accent-ink, not --void');
  });

  test('a present extra overrides its fall-back', () {
    final overridden = ThemeVars(
      surface: probe.surface,
      ink: probe.ink,
      dim: probe.dim,
      line: probe.line,
      bolt: probe.bolt,
      boltHi: probe.boltHi,
      good: probe.good,
      warn: probe.warn,
      again: probe.again,
      noteBorder: probe.noteBorder,
      noteInk: probe.noteInk,
      text: const Color(0xFFEEDDCC),
      faint: const Color(0xFFBBAA99),
      accentInk: const Color(0xFF998877),
    );
    final data = themeFromVars(overridden, Brightness.light);
    final tokens = data.extension<AlixTokens>()!;
    expect(tokens.text, const Color(0xFFEEDDCC));
    expect(tokens.faint, const Color(0xFFBBAA99));
    expect(data.colorScheme.onSecondary, const Color(0xFF998877));
  });
}
