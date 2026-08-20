import 'dart:ui';

import 'package:alix_mobile/review/review_models.dart';

/// A region's rectangle in source pixels, whatever its authored unit.
Rect regionSourceRect(ReviewRegionModel r, double sw, double sh) =>
    r.unit == '%'
    ? Rect.fromLTWH(
        (r.x / 100) * sw,
        (r.y / 100) * sh,
        (r.width / 100) * sw,
        (r.height / 100) * sh,
      )
    : Rect.fromLTWH(r.x, r.y, r.width, r.height);

/// The crop viewport in source pixels; a missing crop is the full source.
Rect cropSourceRect(ReviewCropModel? crop, double sw, double sh) {
  if (crop == null) return Rect.fromLTWH(0, 0, sw, sh);
  return crop.unit == '%'
      ? Rect.fromLTWH(
          (crop.x / 100) * sw,
          (crop.y / 100) * sh,
          (crop.width / 100) * sw,
          (crop.height / 100) * sh,
        )
      : Rect.fromLTWH(crop.x, crop.y, crop.width, crop.height);
}

/// Partial overlap is valid geometry: the mask clips at the source edge
/// instead of floating over non-source space. Null when nothing of the
/// region lies inside the source.
Rect? clipToSource(Rect rect, double sw, double sh) {
  final clipped = rect.intersect(Rect.fromLTWH(0, 0, sw, sh));
  return clipped.width > 0 && clipped.height > 0 ? clipped : null;
}

/// An answered card lifts its reveal-on-answer masks; sibling masks stay.
List<ReviewRegionModel> keptRegions(
  List<ReviewRegionModel> regions,
  bool answered,
) => regions.where((r) => !(answered && r.revealOnAnswer)).toList();

/// The three-role vocabulary shared with the web clients: an asked region
/// shows the blank glyph, a sibling card's mask the hidden glyph, and a
/// cover stays a plain glyphless fill (it hides answer-giving content and
/// is never a question).
String? maskGlyph(ReviewRegionRole role) => switch (role) {
  ReviewRegionRole.asked => '⍰',
  ReviewRegionRole.mask => '⬚',
  ReviewRegionRole.cover => null,
};
