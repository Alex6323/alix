import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show Uint64List;

import 'package:alix_mobile/bridge/review_bridge.dart';
import 'package:alix_mobile/src/rust/api/review.dart' as bridge;

void main() {
  test('a cited bridge excerpt reaches ReviewCardModel', () {
    final card = reviewCardFromBridge(
      bridge.CardView(
        front: 'question',
        frontRuns: const [],
        frontSteps: const [],
        holeSteps: Uint64List(0),
        sectionContext: const [],
        sectionContextRuns: const [],
        sectionContextUnits: const [],
        back: const ['answer'],
        backRuns: const [],
        backUnits: const [],
        answerSteps: const [],
        reshaped: false,
        note: const [],
        hints: const [],
        images: const [],
        imagesBack: const [],
        citations: [
          bridge.CitationView(
            locator: 'src:source.rs:7',
            excerpt: bridge.ExcerptView(
              path: 'source.rs',
              lines: [
                bridge.ExcerptLine(n: BigInt.from(7), text: 'return evidence;'),
              ],
              truncated: false,
            ),
          ),
        ],
      ),
    );

    expect(card.citations, hasLength(1));
    expect(card.citations.single.locator, 'src:source.rs:7');
    expect(card.citations.single.excerpt!.path, 'source.rs');
    expect(card.citations.single.excerpt!.lines.single.number, 7);
    expect(
      card.citations.single.excerpt!.lines.single.text,
      'return evidence;',
    );
  });
}
