import 'package:flutter/material.dart';

/// The alix look, ported from the web app's theme gallery. The token values
/// and their names mirror the CSS custom properties in assets/web/theme.css
/// (the reference for both surfaces): grep a hex here and you find it there.
/// A theme is a var-map (`ThemeVars`) fed through `themeFromVars`, mirrors
/// the CSS's `[data-theme="id"]` blocks; `test/theme_gallery_test.dart`
/// parses those blocks at test time and asserts the two never drift apart.

/// Design tokens Material has no role for.
@immutable
class AlixTokens extends ThemeExtension<AlixTokens> {
  const AlixTokens({
    required this.good,
    required this.warn,
    required this.again,
    required this.bolt,
    required this.boltHi,
    required this.line,
    required this.dim,
    required this.faint,
    required this.text,
    required this.noteBorder,
    required this.noteInk,
  });

  /// The grade trio (--good / --warn / --again): pass green, partial
  /// amber, fail red; always tinted, like the web's legend chips.
  final Color good;
  final Color warn;
  final Color again;

  /// The accent pair (--bolt / --bolt-hi): due dots, focus, small labels.
  final Color bolt;
  final Color boltHi;

  /// Hairline borders (--line), muted ink (--dim), the faintest labels
  /// (--faint: option numbers, the mode tag), and body text a touch softer
  /// than the primary ink (--text: option/chip labels).
  final Color line;
  final Color dim;
  final Color faint;
  final Color text;

  /// The note block's warm pair (--note-border / --note-ink).
  final Color noteBorder;
  final Color noteInk;

  @override
  AlixTokens copyWith({
    Color? good,
    Color? warn,
    Color? again,
    Color? bolt,
    Color? boltHi,
    Color? line,
    Color? dim,
    Color? faint,
    Color? text,
    Color? noteBorder,
    Color? noteInk,
  }) {
    return AlixTokens(
      good: good ?? this.good,
      warn: warn ?? this.warn,
      again: again ?? this.again,
      bolt: bolt ?? this.bolt,
      boltHi: boltHi ?? this.boltHi,
      line: line ?? this.line,
      dim: dim ?? this.dim,
      faint: faint ?? this.faint,
      text: text ?? this.text,
      noteBorder: noteBorder ?? this.noteBorder,
      noteInk: noteInk ?? this.noteInk,
    );
  }

  @override
  AlixTokens lerp(AlixTokens? other, double t) {
    if (other == null) {
      return this;
    }
    Color mix(Color a, Color b) => Color.lerp(a, b, t) ?? b;
    return AlixTokens(
      good: mix(good, other.good),
      warn: mix(warn, other.warn),
      again: mix(again, other.again),
      bolt: mix(bolt, other.bolt),
      boltHi: mix(boltHi, other.boltHi),
      line: mix(line, other.line),
      dim: mix(dim, other.dim),
      faint: mix(faint, other.faint),
      text: mix(text, other.text),
      noteBorder: mix(noteBorder, other.noteBorder),
      noteInk: mix(noteInk, other.noteInk),
    );
  }
}

/// Sugar for widgets: `Theme.of(context).alix.good`. Falls back to the
/// dark tokens when a bare ThemeData carries no extension (plain test
/// pumps), so token reads never crash.
extension AlixThemeTokens on ThemeData {
  AlixTokens get alix => extension<AlixTokens>() ?? _tokensFromVars(_darkVars);
}

/// The flat orange wordmark, the web app's header brand (never reskinned).
/// Used as the AppBar title on every screen.
class AlixWordmark extends StatelessWidget {
  const AlixWordmark({super.key});

  @override
  Widget build(BuildContext context) {
    return const Text(
      'alix',
      style: TextStyle(
        fontFamily: 'IBM Plex Sans',
        fontWeight: FontWeight.w700,
        fontSize: 20,
        letterSpacing: 0.5,
        color: _brand,
      ),
    );
  }
}

/// --brand / --brand-ink: the one primary action's fill, never reskinned.
const _brand = Color(0xFFF0883C);
const _brandInk = Color(0xFF1A1206);

/// One theme's CSS-var vocabulary (an assets/web/theme.css `[data-theme]`
/// block), ported to Color literals. `text`/`faint`/`accentInk` are the 3
/// of the CSS's 4 optional extras this port consumes (the 4th, --brand-text,
/// has no Dart consumer yet - AlixWordmark's brand color is constant); when
/// a theme omits them, the CSS's own `:root { --text: var(--ink); ... }`
/// aliases resolve against whatever --ink/--dim/--void is cascaded for THAT
/// theme, not literally dark's - so the fall-back here is to this same var
/// map's own ink/dim/surface, never a hardcoded default. Public (not `_`)
/// and `@visibleForTesting` only so theme_test.dart can construct a
/// synthetic omitted-extras case: none of the 4 shipped themes omit any of
/// them in the current CSS, so real data alone never exercises this path.
@immutable
@visibleForTesting
class ThemeVars {
  const ThemeVars({
    required this.surface, // --void
    required this.ink, // --ink
    required this.dim, // --dim
    required this.line, // --line (hex or rgba in the CSS)
    required this.bolt, // --bolt
    required this.boltHi, // --bolt-hi
    required this.good, // --good
    required this.warn, // --warn
    required this.again, // --again
    required this.noteBorder, // --note-border
    required this.noteInk, // --note-ink
    this.text, // --text, falls back to ink
    this.faint, // --faint, falls back to dim
    this.accentInk, // --accent-ink, falls back to surface (--void)
  });

  final Color surface;
  final Color ink;
  final Color dim;
  final Color line;
  final Color bolt;
  final Color boltHi;
  final Color good;
  final Color warn;
  final Color again;
  final Color noteBorder;
  final Color noteInk;
  final Color? text;
  final Color? faint;
  final Color? accentInk;
}

/// alix (dark), theme.css `:root, [data-theme="dark"]`.
const _darkVars = ThemeVars(
  surface: Color(0xFF0F1016),
  ink: Color(0xFFEEF0F5),
  dim: Color(0xFF9096A8),
  line: Color(0x17FFFFFF), // rgba(255, 255, 255, .09)
  bolt: Color(0xFF5FD7E0),
  boltHi: Color(0xFF8CE9EF),
  good: Color(0xFF86C986),
  warn: Color(0xFFE6B45C),
  again: Color(0xFFE88F8F),
  noteBorder: Color(0xFFE6B45C),
  noteInk: Color(0xFFF0DCAE),
  text: Color(0xFFC9CDD8),
  faint: Color(0xFF6B7085),
  accentInk: Color(0xFF08131A),
);

/// alix Light, theme.css `[data-theme="light"]`.
const _lightVars = ThemeVars(
  surface: Color(0xFFF4F4FA),
  ink: Color(0xFF17171F),
  dim: Color(0xFF6B6B7A),
  line: Color(0x21141228), // rgba(20, 18, 40, .13)
  bolt: Color(0xFF0E7C86),
  boltHi: Color(0xFF129AA6),
  good: Color(0xFF138A5B),
  warn: Color(0xFFB9790C),
  again: Color(0xFFD23B34),
  noteBorder: Color(0xFFC98A12),
  noteInk: Color(0xFF6A5117),
  text: Color(0xFF3B3B48),
  faint: Color(0xFF9696A5),
  accentInk: Color(0xFFFFFFFF),
);

/// Nord, theme.css `[data-theme="nord"]`.
const _nordVars = ThemeVars(
  surface: Color(0xFF2E3440),
  ink: Color(0xFFECEFF4),
  dim: Color(0xFFA3ADBA),
  line: Color(0x1AFFFFFF), // rgba(255, 255, 255, .10)
  bolt: Color(0xFF88C0D0),
  boltHi: Color(0xFF88C0D0),
  good: Color(0xFFA3BE8C),
  warn: Color(0xFFEBCB8B),
  again: Color(0xFFBF616A),
  noteBorder: Color(0xFFEBCB8B),
  noteInk: Color(0xFFECEFF4),
  text: Color(0xFFD8DEE9),
  faint: Color(0xFF7B869C),
  accentInk: Color(0xFF10202A),
);

/// Solarized Light, theme.css `[data-theme="solarized-light"]`.
const _solarizedLightVars = ThemeVars(
  surface: Color(0xFFFDF6E3),
  ink: Color(0xFF073642),
  dim: Color(0xFF839496),
  line: Color(0x33504628), // rgba(80, 70, 40, .20)
  bolt: Color(0xFF268BD2),
  boltHi: Color(0xFF268BD2),
  good: Color(0xFF859900),
  warn: Color(0xFFB58900),
  again: Color(0xFFDC322F),
  noteBorder: Color(0xFFB58900),
  noteInk: Color(0xFF073642),
  text: Color(0xFF586E75),
  faint: Color(0xFF93A1A1),
  accentInk: Color(0xFFFFFFFF),
);

/// A registered theme: id + display name mirror theme.js's THEMES tuple
/// (minus the web-only kids trio); `data` is the built ThemeData.
@immutable
class AlixTheme {
  const AlixTheme({
    required this.id,
    required this.name,
    required this.mode,
    required this.data,
  });

  final String id;
  final String name;
  final Brightness mode;
  final ThemeData data;
}

/// The theme gallery. Ported from assets/web/theme.css / theme.js's
/// non-kids THEMES entries; test/theme_gallery_test.dart parses those files
/// at test time and asserts these hexes match. Only 4 shipped here
/// (dark/light/nord/solarized-light) - the remaining ~14 land as data in a
/// follow-up task, without needing any change to the mapper or the test.
final List<AlixTheme> alixThemes = [
  AlixTheme(
    id: 'dark',
    name: 'alix',
    mode: Brightness.dark,
    data: themeFromVars(_darkVars, Brightness.dark),
  ),
  AlixTheme(
    id: 'light',
    name: 'alix Light',
    mode: Brightness.light,
    data: themeFromVars(_lightVars, Brightness.light),
  ),
  AlixTheme(
    id: 'nord',
    name: 'Nord',
    mode: Brightness.dark,
    data: themeFromVars(_nordVars, Brightness.dark),
  ),
  AlixTheme(
    id: 'solarized-light',
    name: 'Solarized Light',
    mode: Brightness.light,
    data: themeFromVars(_solarizedLightVars, Brightness.light),
  ),
];

/// Looks up a registered theme's ThemeData by id; an unknown or null id
/// falls back to the dark default.
ThemeData themeById(String? id) {
  for (final theme in alixThemes) {
    if (theme.id == id) return theme.data;
  }
  return alixThemes.first.data;
}

/// The default dark palette ("alix" in the web gallery).
ThemeData alixDark() => themeFromVars(_darkVars, Brightness.dark);

/// The default light palette ("alix Light").
ThemeData alixLight() => themeFromVars(_lightVars, Brightness.light);

AlixTokens _tokensFromVars(ThemeVars v) => AlixTokens(
  good: v.good,
  warn: v.warn,
  again: v.again,
  bolt: v.bolt,
  boltHi: v.boltHi,
  line: v.line,
  dim: v.dim,
  faint: v.faint ?? v.dim,
  text: v.text ?? v.ink,
  noteBorder: v.noteBorder,
  noteInk: v.noteInk,
);

/// Builds a full ThemeData from a var map: AlixTokens plus a ColorScheme
/// (--brand/--brand-ink are constants, never sourced from the var map). The
/// Material fields the CSS has no var for (onError/errorContainer/
/// onErrorContainer) are derived from --again + --void/--ink, sensible and
/// readable but not drift-guarded. Public and `@visibleForTesting` (see
/// `ThemeVars`) so the fall-back cascade can be unit-tested directly; every
/// theme in `alixThemes` still goes through this same function.
@visibleForTesting
ThemeData themeFromVars(ThemeVars v, Brightness mode) {
  final tokens = _tokensFromVars(v);
  final accentInk = v.accentInk ?? v.surface;
  final onError = mode == Brightness.dark ? v.surface : const Color(0xFFFFFFFF);
  final errorContainer = Color.lerp(v.surface, v.again, 0.22)!;
  final onErrorContainer = Color.lerp(v.ink, v.again, 0.55)!;
  final scheme = ColorScheme(
    brightness: mode,
    primary: _brand,
    onPrimary: _brandInk,
    secondary: v.bolt,
    onSecondary: accentInk,
    error: v.again,
    onError: onError,
    errorContainer: errorContainer,
    onErrorContainer: onErrorContainer,
    surface: v.surface,
    onSurface: v.ink,
    onSurfaceVariant: v.dim,
    outline: v.line,
  );
  return _theme(scheme, tokens);
}

/// One flat surface, hairline ghosts, the brand chip as the sole filled
/// action, and the mono terminal-lineage header.
ThemeData _theme(ColorScheme scheme, AlixTokens tokens) {
  final radius = RoundedRectangleBorder(borderRadius: BorderRadius.circular(10));
  return ThemeData(
    colorScheme: scheme,
    scaffoldBackgroundColor: scheme.surface,
    fontFamily: 'IBM Plex Sans',
    appBarTheme: AppBarTheme(
      backgroundColor: scheme.surface,
      foregroundColor: tokens.dim,
      elevation: 0,
      scrolledUnderElevation: 0,
      titleTextStyle: TextStyle(
        fontFamily: 'IBM Plex Mono',
        fontSize: 15,
        color: tokens.dim,
      ),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(shape: radius),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        shape: radius,
        side: BorderSide(color: tokens.line),
        foregroundColor: scheme.onSurface,
      ),
    ),
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(shape: radius),
    ),
    extensions: [tokens],
  );
}
