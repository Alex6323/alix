import 'package:flutter/material.dart';

import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/theme.dart';

const _sans = 'IBM Plex Sans';

class CrumbStrip extends StatelessWidget {
  const CrumbStrip({super.key, required this.crumb});

  final ReviewCrumbModel crumb;

  static const double height = 40;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final ink = theme.colorScheme.onSurface;
    final tokens = theme.alix;
    return SizedBox(
      height: height,
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        padding: const EdgeInsets.fromLTRB(20, 10, 20, 6),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            for (final (index, name) in crumb.regions.indexed) ...[
              if (index > 0) const SizedBox(width: 14),
              _region(
                name,
                index == crumb.current,
                index < crumb.cells.length
                    ? crumb.cells[index]
                    : const <String>[],
                ink,
                tokens,
              ),
            ],
          ],
        ),
      ),
    );
  }

  Widget _region(
    String name,
    bool current,
    Iterable<String> cells,
    Color ink,
    AlixTokens tokens,
  ) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SizedBox(
          height: 16,
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 140),
            child: Text(
              name,
              maxLines: 1,
              softWrap: false,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontFamily: _sans,
                fontSize: 10.5,
                letterSpacing: 0.3,
                height: 1.2,
                color: ink.withValues(alpha: current ? 1 : 0.5),
                fontWeight: current ? FontWeight.w600 : FontWeight.w400,
              ),
            ),
          ),
        ),
        const SizedBox(height: 3),
        Row(
          mainAxisSize: MainAxisSize.min,
          children: [for (final tier in cells) _cell(tier, ink, tokens)],
        ),
      ],
    );
  }

  static const Color _retired = Color(0xFFA48FD8);

  Widget _cell(String tier, Color ink, AlixTokens tokens) {
    final Color fill = switch (tier) {
      'seen' => ink.withValues(alpha: 0.55),
      'learning' => ink.withValues(alpha: 0.85),
      'learned-strong' => tokens.good,
      'learned-fading' => tokens.warn,
      'learned-weak' => tokens.again,
      'retired' => _retired,
      _ => ink.withValues(alpha: 0.22),
    };
    return Container(
      width: 5,
      height: 3,
      margin: const EdgeInsets.only(right: 1),
      decoration: BoxDecoration(
        color: fill,
        borderRadius: BorderRadius.circular(1),
      ),
    );
  }
}
