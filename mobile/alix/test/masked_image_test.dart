import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/review/mask_geometry.dart';
import 'package:alix_mobile/review/masked_image.dart';
import 'package:alix_mobile/review/review_models.dart';

ReviewRegionModel region({
  ReviewRegionRole role = ReviewRegionRole.asked,
  bool revealOnAnswer = true,
  double x = 10,
  double y = 10,
  double width = 20,
  double height = 20,
  String unit = 'px',
}) => ReviewRegionModel(
  role: role,
  revealOnAnswer: revealOnAnswer,
  x: x,
  y: y,
  width: width,
  height: height,
  unit: unit,
);

Future<ui.Image> whiteSource(int w, int h) {
  final recorder = ui.PictureRecorder();
  ui.Canvas(recorder).drawRect(
    Rect.fromLTWH(0, 0, w.toDouble(), h.toDouble()),
    Paint()..color = const Color(0xFFFFFFFF),
  );
  return recorder.endRecording().toImage(w, h);
}

Future<ByteData> paintToPixels(
  MaskPainter painter,
  double width,
  double height,
) async {
  final recorder = ui.PictureRecorder();
  painter.paint(ui.Canvas(recorder), Size(width, height));
  final image = await recorder.endRecording().toImage(
    width.round(),
    height.round(),
  );
  final data = await image.toByteData();
  image.dispose();
  return data!;
}

Color pixel(ByteData data, int width, int x, int y) {
  final offset = (y * width + x) * 4;
  return Color.fromARGB(
    data.getUint8(offset + 3),
    data.getUint8(offset),
    data.getUint8(offset + 1),
    data.getUint8(offset + 2),
  );
}

/// Resolves synchronously with a prebuilt image, so widget tests need no
/// real decode and no runAsync.
class SyncImage extends ImageProvider<SyncImage> {
  SyncImage(this.image);

  final ui.Image image;

  @override
  Future<SyncImage> obtainKey(ImageConfiguration configuration) =>
      SynchronousFuture(this);

  @override
  ImageStreamCompleter loadImage(SyncImage key, ImageDecoderCallback decode) =>
      OneFrameImageStreamCompleter(
        SynchronousFuture(ImageInfo(image: image.clone())),
      );
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('geometry laws', () {
    test('a region maps to source pixels under either unit', () {
      final cases = {
        'px': region(x: 10, y: 20, width: 30, height: 40),
        '%': region(x: 5, y: 10, width: 15, height: 20, unit: '%'),
      };
      final expected = {
        'px': const Rect.fromLTWH(10, 20, 30, 40),
        '%': const Rect.fromLTWH(10, 20, 30, 40),
      };
      for (final entry in cases.entries) {
        expect(
          regionSourceRect(entry.value, 200, 200),
          expected[entry.key],
          reason: 'unit ${entry.key} on a 200x200 source',
        );
      }
    });

    test('a missing crop is the full source and an authored one converts', () {
      expect(
        cropSourceRect(null, 200, 100),
        const Rect.fromLTWH(0, 0, 200, 100),
      );
      const px = ReviewCropModel(
        x: 80,
        y: 0,
        width: 40,
        height: 100,
        unit: 'px',
      );
      expect(cropSourceRect(px, 100, 100), const Rect.fromLTWH(80, 0, 40, 100));
      const pct = ReviewCropModel(
        x: 40,
        y: 0,
        width: 20,
        height: 100,
        unit: '%',
      );
      expect(
        cropSourceRect(pct, 200, 100),
        const Rect.fromLTWH(80, 0, 40, 100),
      );
    });

    test(
      'clipping keeps inside, cuts overlap at the edge, and voids outside',
      () {
        final cases = <(Rect, Rect?)>[
          (
            const Rect.fromLTWH(10, 10, 20, 20),
            const Rect.fromLTWH(10, 10, 20, 20),
          ),
          (
            const Rect.fromLTWH(90, 70, 20, 20),
            const Rect.fromLTWH(90, 70, 10, 20),
          ),
          (const Rect.fromLTWH(120, 10, 20, 20), null),
          (const Rect.fromLTWH(100, 10, 20, 20), null),
        ];
        for (final (rect, want) in cases) {
          expect(
            clipToSource(rect, 100, 100),
            want,
            reason: 'clipping $rect against a 100x100 source',
          );
        }
      },
    );

    test('every role and reveal flag lifts exactly per reveal_on_answer', () {
      for (final role in ReviewRegionRole.values) {
        for (final reveals in [true, false]) {
          for (final answered in [true, false]) {
            final r = region(role: role, revealOnAnswer: reveals);
            final kept = keptRegions([r], answered);
            expect(
              kept.length,
              answered && reveals ? 0 : 1,
              reason: 'role $role revealOnAnswer $reveals answered $answered',
            );
          }
        }
      }
    });

    test('the glyph vocabulary matches the web clients', () {
      expect(maskGlyph(ReviewRegionRole.asked), '⍰');
      expect(maskGlyph(ReviewRegionRole.mask), '⬚');
      expect(maskGlyph(ReviewRegionRole.cover), isNull);
    });
  });

  group('painter laws', () {
    // Glyph ink is not falsifiable here: the test font maps neither mask
    // codepoint, so an asked center reads as pure fill. The vocabulary law
    // above pins the role mapping; the device look is checked visually.
    test('a mask paints the plain fill over the image', () async {
      final source = await whiteSource(100, 100);
      final painter = MaskPainter(
        source: source,
        regions: [
          region(
            role: ReviewRegionRole.cover,
            x: 0,
            y: 0,
            width: 50,
            height: 100,
          ),
        ],
        crop: const Rect.fromLTWH(0, 0, 100, 100),
      );
      final data = await paintToPixels(painter, 100, 100);
      expect(
        pixel(data, 100, 25, 50),
        MaskPainter.fill,
        reason: 'inside the mask the fill covers the image',
      );
      expect(
        pixel(data, 100, 75, 50),
        const Color(0xFFFFFFFF),
        reason: 'outside the mask the source shows through',
      );
      source.dispose();
    });

    test('the cropped path keeps a mask clipped at the source edge', () async {
      final source = await whiteSource(100, 100);
      final painter = MaskPainter(
        source: source,
        regions: [
          region(
            role: ReviewRegionRole.cover,
            x: 90,
            y: 10,
            width: 20,
            height: 20,
          ),
        ],
        crop: const Rect.fromLTWH(80, 0, 40, 100),
      );
      final data = await paintToPixels(painter, 200, 500);
      expect(
        pixel(data, 200, 74, 100),
        MaskPainter.fill,
        reason: 'the in-source mask half paints inside the viewport',
      );
      expect(
        pixel(data, 200, 110, 100).a,
        0,
        reason: 'crop space beyond the bitmap stays empty, mask included',
      );
      source.dispose();
    });
  });

  group('widget laws', () {
    Future<void> pump(
      WidgetTester tester,
      ui.Image source,
      ReviewImageModel image, {
      bool answered = false,
      VoidCallback? onAskedGone,
    }) async {
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: MaskedCardImage(
              provider: SyncImage(source),
              image: image,
              answered: answered,
              height: 180,
              onAskedGone: onAskedGone,
            ),
          ),
        ),
      );
      await tester.pump();
    }

    MaskPainter painterOf(WidgetTester tester) {
      final paint = tester.widget<CustomPaint>(
        find.byWidgetPredicate(
          (w) => w is CustomPaint && w.painter is MaskPainter,
        ),
      );
      return paint.painter! as MaskPainter;
    }

    testWidgets('answering lifts reveal-on-answer masks and keeps siblings', (
      tester,
    ) async {
      final source = await whiteSource(100, 100);
      final image = ReviewImageModel(
        src: 'unused',
        regions: [
          region(x: 5, y: 5),
          region(
            role: ReviewRegionRole.mask,
            revealOnAnswer: false,
            x: 35,
            y: 5,
          ),
        ],
      );
      await pump(tester, source, image);
      expect(painterOf(tester).regions, hasLength(2));
      await pump(tester, source, image, answered: true);
      final kept = painterOf(tester).regions;
      expect(kept, hasLength(1), reason: 'the asked mask lifts on answer');
      expect(kept.single.role, ReviewRegionRole.mask);
      source.dispose();
    });

    testWidgets('a wholly out-of-source asked region reports exactly once', (
      tester,
    ) async {
      final source = await whiteSource(100, 100);
      var fired = 0;
      await pump(
        tester,
        source,
        ReviewImageModel(src: 'unused', regions: [region(x: 120, y: 10)]),
        onAskedGone: () => fired += 1,
      );
      await tester.pump();
      expect(fired, 1, reason: 'the loud report fires');
      await tester.pump();
      expect(fired, 1, reason: 'and never repeats');
      source.dispose();
    });

    testWidgets('an in-source asked region never reports', (tester) async {
      final source = await whiteSource(100, 100);
      var fired = 0;
      await pump(
        tester,
        source,
        ReviewImageModel(src: 'unused', regions: [region(x: 90, y: 70)]),
        onAskedGone: () => fired += 1,
      );
      await tester.pump();
      expect(fired, 0, reason: 'edge overlap is valid geometry, not an error');
      source.dispose();
    });

    testWidgets('a cropped image sizes to the crop viewport', (tester) async {
      final source = await whiteSource(100, 100);
      await pump(
        tester,
        source,
        const ReviewImageModel(
          src: 'unused',
          crop: ReviewCropModel(
            x: 80,
            y: 0,
            width: 40,
            height: 100,
            unit: 'px',
          ),
        ),
      );
      final box = tester.getSize(find.byType(AspectRatio));
      expect(
        box.width / box.height,
        moreOrLessEquals(0.4),
        reason: 'the viewport carries the crop aspect, not the source aspect',
      );
      source.dispose();
    });
  });
}
