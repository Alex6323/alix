// The drift guard: at test time this reads the WEB gallery's source of
// truth (assets/web/theme.css + theme.js) and asserts the Dart registry
// (lib/theme.dart's alixThemes) carries the same hexes. A wrong ported hex
// fails a test here, not just a code review - and the parser is general
// (all 18 non-kids ids, not just the 4 shipped so far), so a future task
// that adds more themes to alixThemes as pure data is covered by this same
// test with no changes needed.
//
// flutter test's cwd is apps/mobile (verified: Directory.current.path during
// a run), so the repo root is two levels up.
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/theme.dart';

const _cssPath = '../../assets/web/theme.css';
const _jsPath = '../../assets/web/theme.js';

/// Parses theme.js's `THEMES` array for {id, mode}, excluding `mode:
/// "kids"` (the kids app's own trio isn't part of this adult gallery).
Set<String> _parseNonKidsThemeIds(String js) {
  final start = js.indexOf('var THEMES');
  final open = js.indexOf('[', start);
  final close = js.indexOf('];', open);
  final body = js.substring(open, close);
  final entry = RegExp(
    r'id:\s*"([^"]+)"\s*,\s*name:\s*"[^"]*"\s*,\s*mode:\s*"([^"]+)"',
  );
  final ids = <String>{};
  for (final m in entry.allMatches(body)) {
    if (m.group(2) != 'kids') ids.add(m.group(1)!);
  }
  return ids;
}

/// Parses theme.css into {theme-id: {var-name: raw-value}}: one entry per
/// `[data-theme="id"] { ... }` ruleset (a combined selector like `:root,
/// [data-theme="dark"]` counts for "dark"). Values are kept as raw CSS text
/// (hex, rgba(...), or otherwise, e.g. a color-mix() call) - only the keys a
/// caller actually looks up get resolved to a Color, so declarations this
/// port doesn't consume (--note-bg, --panel-top, ...) never need parsing.
/// No nested `{}` appears inside a theme's own var block in this file (only
/// the unrelated @font-face/@media rules do, and those never mention
/// `[data-theme=...]`), so a simple non-nesting brace match is sufficient.
Map<String, Map<String, String>> _parseCssBlocks(String css) {
  final ruleset = RegExp(r'([^{}]+)\{([^}]*)\}');
  final varPair = RegExp(r'--([a-z0-9-]+)\s*:\s*([^;]+);');
  final idInSelector = RegExp(r'\[data-theme="([a-z0-9-]+)"\]');
  final blocks = <String, Map<String, String>>{};
  for (final rule in ruleset.allMatches(css)) {
    final id = idInSelector.firstMatch(rule.group(1)!)?.group(1);
    if (id == null) continue;
    final vars = <String, String>{};
    for (final v in varPair.allMatches(rule.group(2)!)) {
      vars[v.group(1)!] = v.group(2)!.trim();
    }
    blocks[id] = vars;
  }
  return blocks;
}

/// Parses one CSS color literal: `#rgb`, `#rrggbb`, or `rgba(r, g, b, a)`
/// (`rgb(...)`, no alpha, also accepted). --line is the only var that uses
/// rgba() today, but both forms are handled generally.
Color _parseCssColor(String raw) {
  final value = raw.trim();
  if (value.startsWith('#')) {
    var hex = value.substring(1);
    if (hex.length == 3) {
      hex = hex.split('').map((c) => '$c$c').join();
    }
    return Color(0xFF000000 | int.parse(hex, radix: 16));
  }
  final rgba = RegExp(
    r'rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*([\d.]+)\s*)?\)',
  ).firstMatch(value);
  if (rgba != null) {
    final r = int.parse(rgba.group(1)!);
    final g = int.parse(rgba.group(2)!);
    final b = int.parse(rgba.group(3)!);
    final a = rgba.group(4) == null ? 1.0 : double.parse(rgba.group(4)!);
    return Color.fromARGB((a * 255).round(), r, g, b);
  }
  throw FormatException('unparseable CSS color: $raw');
}

void main() {
  final css = File(_cssPath).readAsStringSync();
  final js = File(_jsPath).readAsStringSync();
  final nonKidsIds = _parseNonKidsThemeIds(js);
  final cssBlocks = _parseCssBlocks(css);

  test('theme.js lists 18 non-kids theme ids', () {
    expect(nonKidsIds.length, 18,
        reason: 'THEMES in theme.js changed shape; re-check the kids filter');
  });

  test('every seed theme id is a real non-kids id in theme.js', () {
    for (final theme in alixThemes) {
      expect(nonKidsIds, contains(theme.id));
    }
  });

  for (final theme in alixThemes) {
    group('${theme.id}: registry matches assets/web/theme.css', () {
      final raw = cssBlocks[theme.id];

      test('theme.css has a [data-theme="${theme.id}"] block', () {
        expect(raw, isNotNull);
      });

      test('tokens + scheme carry the CSS-direct colors', () {
        Color core(String key) => _parseCssColor(raw![key]!);
        Color extra(String key, String fallback) =>
            raw!.containsKey(key) ? _parseCssColor(raw[key]!) : core(fallback);

        final scheme = theme.data.colorScheme;
        final tokens = theme.data.extension<AlixTokens>()!;

        expect(tokens.good, core('good'), reason: '--good');
        expect(tokens.warn, core('warn'), reason: '--warn');
        expect(tokens.again, core('again'), reason: '--again');
        expect(tokens.bolt, core('bolt'), reason: '--bolt');
        expect(tokens.boltHi, core('bolt-hi'), reason: '--bolt-hi');
        expect(tokens.line, core('line'), reason: '--line');
        expect(tokens.dim, core('dim'), reason: '--dim');
        expect(tokens.faint, extra('faint', 'dim'), reason: '--faint');
        expect(tokens.text, extra('text', 'ink'), reason: '--text');
        expect(tokens.noteBorder, core('note-border'), reason: '--note-border');
        expect(tokens.noteInk, core('note-ink'), reason: '--note-ink');

        expect(scheme.surface, core('void'), reason: '--void');
        expect(scheme.onSurface, core('ink'), reason: '--ink');
        expect(scheme.secondary, core('bolt'), reason: '--bolt');
        expect(scheme.onSecondary, extra('accent-ink', 'void'),
            reason: '--accent-ink');
        expect(scheme.error, core('again'), reason: '--again');
        expect(scheme.outline, core('line'), reason: '--line');
        expect(scheme.onSurfaceVariant, core('dim'), reason: '--dim');
      });

      test('brightness matches theme.js mode', () {
        expect(
          theme.data.colorScheme.brightness,
          theme.mode,
        );
      });
    });
  }
}
