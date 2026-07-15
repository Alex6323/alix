import 'package:flutter/material.dart';

/// The alix look, ported from the web app's default palettes. The token
/// values and their names mirror the CSS custom properties in
/// assets/web/theme.css (the reference for both surfaces): grep a hex here
/// and you find it there. Two palettes only; the web's theme gallery is a
/// web feature.

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

  /// Hairline borders (--line) and muted ink (--dim).
  final Color line;
  final Color dim;

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
      noteBorder: mix(noteBorder, other.noteBorder),
      noteInk: mix(noteInk, other.noteInk),
    );
  }
}

/// Sugar for widgets: `Theme.of(context).alix.good`. Falls back to the
/// dark tokens when a bare ThemeData carries no extension (plain test
/// pumps), so token reads never crash.
extension AlixThemeTokens on ThemeData {
  AlixTokens get alix => extension<AlixTokens>() ?? _darkTokens;
}

/// --brand / --brand-ink: the one primary action's fill, never reskinned.
const _brand = Color(0xFFF0883C);
const _brandInk = Color(0xFF1A1206);

const _darkTokens = AlixTokens(
  good: Color(0xFF86C986),
  warn: Color(0xFFE6B45C),
  again: Color(0xFFE88F8F),
  bolt: Color(0xFF5FD7E0),
  boltHi: Color(0xFF8CE9EF),
  line: Color(0x17FFFFFF),
  dim: Color(0xFF9096A8),
  noteBorder: Color(0xFFE6B45C),
  noteInk: Color(0xFFF0DCAE),
);

const _lightTokens = AlixTokens(
  good: Color(0xFF138A5B),
  warn: Color(0xFFB9790C),
  again: Color(0xFFD23B34),
  bolt: Color(0xFF0E7C86),
  boltHi: Color(0xFF129AA6),
  line: Color(0x21141228),
  dim: Color(0xFF6B6B7A),
  noteBorder: Color(0xFFC98A12),
  noteInk: Color(0xFF6A5117),
);

/// The default dark palette ("alix" in the web gallery).
ThemeData alixDark() {
  const scheme = ColorScheme(
    brightness: Brightness.dark,
    primary: _brand,
    onPrimary: _brandInk,
    secondary: Color(0xFF5FD7E0),
    onSecondary: Color(0xFF08131A),
    error: Color(0xFFE88F8F),
    onError: Color(0xFF0F1016),
    errorContainer: Color(0xFF3A2328),
    onErrorContainer: Color(0xFFF0DCDC),
    surface: Color(0xFF0F1016),
    onSurface: Color(0xFFEEF0F5),
    onSurfaceVariant: Color(0xFF9096A8),
    outline: Color(0x17FFFFFF),
  );
  return _theme(scheme, _darkTokens);
}

/// The default light palette ("alix Light").
ThemeData alixLight() {
  const scheme = ColorScheme(
    brightness: Brightness.light,
    primary: _brand,
    onPrimary: _brandInk,
    secondary: Color(0xFF0E7C86),
    onSecondary: Color(0xFFFFFFFF),
    error: Color(0xFFD23B34),
    onError: Color(0xFFFFFFFF),
    errorContainer: Color(0xFFF6DDDA),
    onErrorContainer: Color(0xFF5A1512),
    surface: Color(0xFFF4F4FA),
    onSurface: Color(0xFF17171F),
    onSurfaceVariant: Color(0xFF6B6B7A),
    outline: Color(0x21141228),
  );
  return _theme(scheme, _lightTokens);
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
