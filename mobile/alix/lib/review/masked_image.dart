import 'dart:ui' as ui;

import 'package:flutter/material.dart';

import 'package:alix_mobile/review/mask_geometry.dart';
import 'package:alix_mobile/review/review_models.dart';

/// A card image drawn with its region masks (ADR 0034), sharing the web
/// clients' vocabulary: the asked region shows the blank glyph, a sibling
/// card's mask the hidden glyph, and a cover stays a plain fill. A crop is
/// a viewport: the full image shifts inside it while regions stay in
/// full-source space, so a mask never paints into non-source crop space.
class MaskedCardImage extends StatefulWidget {
  const MaskedCardImage({
    super.key,
    required this.provider,
    required this.image,
    required this.answered,
    required this.height,
    this.onAskedGone,
  });

  final ImageProvider provider;
  final ReviewImageModel image;
  final bool answered;
  final double height;

  /// Fired once when an asked region clips to nothing against the source:
  /// a question about nothing visible is broken and fails loud. Empty
  /// sibling masks and covers hide nothing that exists and stay silent.
  final VoidCallback? onAskedGone;

  @override
  State<MaskedCardImage> createState() => _MaskedCardImageState();
}

class _MaskedCardImageState extends State<MaskedCardImage> {
  ImageStream? _stream;
  ImageStreamListener? _listener;
  ImageInfo? _info;
  bool _reported = false;

  @override
  void initState() {
    super.initState();
    _resolve();
  }

  @override
  void didUpdateWidget(MaskedCardImage old) {
    super.didUpdateWidget(old);
    if (old.provider != widget.provider) {
      // The old card's pixels must not paint under the new card's masks:
      // back to the empty pre-size state until the new source resolves.
      _reported = false;
      _info?.dispose();
      _info = null;
      _resolve();
    } else if (!identical(old.image, widget.image)) {
      // Same source, next derived card: its own regions need their own
      // asked-gone check against the already resolved image.
      _reported = false;
      final info = _info;
      if (info != null) _checkAskedGone(info.image);
    }
  }

  void _resolve() {
    _detach();
    final stream = widget.provider.resolve(ImageConfiguration.empty);
    final listener = ImageStreamListener((info, synchronousCall) {
      if (!mounted) {
        info.dispose();
        return;
      }
      // A synchronous delivery arrives inside initState/didUpdateWidget,
      // where setState is illegal; the frame being built paints it anyway.
      if (synchronousCall) {
        _info?.dispose();
        _info = info;
      } else {
        setState(() {
          _info?.dispose();
          _info = info;
        });
      }
      _checkAskedGone(info.image);
    });
    _stream = stream;
    _listener = listener;
    stream.addListener(listener);
  }

  void _checkAskedGone(ui.Image source) {
    if (_reported || widget.onAskedGone == null) return;
    final sw = source.width.toDouble(), sh = source.height.toDouble();
    final gone = widget.image.regions.any(
      (r) =>
          r.role == ReviewRegionRole.asked &&
          clipToSource(regionSourceRect(r, sw, sh), sw, sh) == null,
    );
    if (!gone) return;
    _reported = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) widget.onAskedGone?.call();
    });
  }

  void _detach() {
    if (_stream != null && _listener != null) {
      _stream!.removeListener(_listener!);
    }
    _stream = null;
    _listener = null;
  }

  @override
  void dispose() {
    _detach();
    _info?.dispose();
    _info = null;
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final info = _info;
    // Nothing renders until the source's size is known; a broken pre-load
    // state would place masks over nothing.
    if (info == null) return const SizedBox.shrink();
    final source = info.image;
    final sw = source.width.toDouble(), sh = source.height.toDouble();
    final crop = cropSourceRect(widget.image.crop, sw, sh);
    return SizedBox(
      height: widget.height,
      child: Center(
        child: AspectRatio(
          aspectRatio: crop.width / crop.height,
          child: Semantics(
            label: widget.image.alt,
            image: true,
            child: ClipRect(
              child: CustomPaint(
                painter: MaskPainter(
                  source: source,
                  regions: keptRegions(widget.image.regions, widget.answered),
                  crop: crop,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// Draws the full source mapped through the crop viewport, then each mask
/// clipped to the source, with the web clients' treatments: one calm fill,
/// role told by the glyph alone (blank for asked, hidden for a sibling
/// mask, none for a cover).
class MaskPainter extends CustomPainter {
  MaskPainter({
    required this.source,
    required this.regions,
    required this.crop,
  });

  final ui.Image source;
  final List<ReviewRegionModel> regions;
  final Rect crop;

  static const fill = Color(0xFF64748B);

  @override
  void paint(Canvas canvas, Size size) {
    final sw = source.width.toDouble(), sh = source.height.toDouble();
    final sx = size.width / crop.width, sy = size.height / crop.height;
    Rect map(Rect r) => Rect.fromLTWH(
      (r.left - crop.left) * sx,
      (r.top - crop.top) * sy,
      r.width * sx,
      r.height * sy,
    );
    canvas.clipRect(Offset.zero & size);
    canvas.drawImageRect(
      source,
      Rect.fromLTWH(0, 0, sw, sh),
      map(Rect.fromLTWH(0, 0, sw, sh)),
      Paint()..filterQuality = FilterQuality.medium,
    );
    for (final r in regions) {
      final clipped = clipToSource(regionSourceRect(r, sw, sh), sw, sh);
      if (clipped == null) continue;
      final dst = map(clipped);
      canvas.drawRRect(
        RRect.fromRectAndRadius(dst, const Radius.circular(4)),
        Paint()..color = fill,
      );
      canvas.drawRRect(
        RRect.fromRectAndRadius(dst, const Radius.circular(4)),
        Paint()
          ..color = const Color(0x40000000)
          ..style = PaintingStyle.stroke,
      );
      final glyph = maskGlyph(r.role);
      if (glyph == null) continue;
      final fontSize = (dst.shortestSide * 0.55).clamp(10.0, 40.0);
      final painter = TextPainter(
        text: TextSpan(
          text: glyph,
          style: TextStyle(
            color: const Color(0xBFFFFFFF),
            fontSize: fontSize,
            height: 1,
          ),
        ),
        textDirection: TextDirection.ltr,
      )..layout();
      painter.paint(
        canvas,
        dst.center - Offset(painter.width / 2, painter.height / 2),
      );
    }
  }

  @override
  bool shouldRepaint(MaskPainter old) =>
      old.source != source || old.regions != regions || old.crop != crop;
}
