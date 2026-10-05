import 'package:flutter/painting.dart';

import 'package:alix_mobile/theme.dart';

const Color _retired = Color(0xFFA48FD8);

Color tierColor(String tier, {required Color ink, required AlixTokens tokens}) {
  return switch (tier) {
    'seen' => ink.withValues(alpha: 0.55),
    'learning' => ink.withValues(alpha: 0.85),
    'learned-strong' => tokens.good,
    'learned-fading' => tokens.warn,
    'learned-weak' => tokens.again,
    'retired' => _retired,
    _ => ink.withValues(alpha: 0.22),
  };
}
