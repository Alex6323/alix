import 'package:flutter_rust_bridge/flutter_rust_bridge.dart'
    show AnyhowException;

import 'package:alix_mobile/bridge/inline_run_bridge.dart';
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/review_port.dart';
import 'package:alix_mobile/src/rust/api/review.dart' as bridge;

class ReviewBridgeFactory implements ReviewPortFactory {
  const ReviewBridgeFactory();

  @override
  ReviewPort open({
    required String deckPath,
    required String rootDir,
    ReviewDepth? depth,
    String? device,
  }) {
    try {
      return ReviewBridgePort(
        bridge.ReviewSession.open(
          deckPath: deckPath,
          rootDir: rootDir,
          depth: _depthToBridge(depth),
          device: device,
        ),
      );
    } catch (error) {
      throw ReviewOpenFailure(
        error is AnyhowException ? error.message : '$error',
      );
    }
  }
}

class ReviewBridgePort implements ReviewPort {
  ReviewBridgePort(this._session);

  final bridge.ReviewSession _session;

  @override
  ReviewStateModel get state => _stateFromBridge(_session.state());

  @override
  ReviewForeignWriterModel? get foreignWriter {
    final writer = _session.foreignWriter();
    return writer == null
        ? null
        : ReviewForeignWriterModel(
            device: writer.device,
            ageMs: writer.ageMs.toInt(),
          );
  }

  @override
  ReviewTutorCardModel? get tutorCard {
    final tutor = _session.tutorCard();
    return tutor == null
        ? null
        : ReviewTutorCardModel(
            subject: tutor.subject,
            front: tutor.front,
            back: tutor.back,
            at: tutor.at,
            line: tutor.line.toInt(),
          );
  }

  @override
  bool get deckHasExam => _session.deckHasExam();

  @override
  ReviewCrumbModel? crumb(int nowMs) {
    final crumb = _session.crumb(nowMs: BigInt.from(nowMs));
    return crumb == null
        ? null
        : ReviewCrumbModel(
            regions: crumb.regions,
            current: crumb.current,
            cells: crumb.cells,
          );
  }

  @override
  ReviewChoiceFeedbackModel? choose(int chosen) {
    final choice = _session.choose(chosen: chosen);
    return choice == null
        ? null
        : ReviewChoiceFeedbackModel(
            chosen: choice.chosen.toInt(),
            correct: choice.correct.toInt(),
            passed: choice.passed,
          );
  }

  @override
  ReviewCheckFeedbackModel? check(List<String> lines) {
    final check = _session.check(lines: lines);
    return check == null
        ? null
        : ReviewCheckFeedbackModel(
            results: [
              for (final result in check.results)
                ReviewTypedResultModel(
                  input: result.input,
                  expected: result.expected,
                  passed: result.passed,
                ),
            ],
            passed: check.passed,
          );
  }

  @override
  ReviewStateModel introduce() => _stateFromBridge(_session.introduce());

  @override
  ReviewStateModel grade(ReviewGrade grade) {
    return _stateFromBridge(_session.grade(grade: _gradeToBridge(grade)));
  }

  @override
  ReviewGrade keypointGrade({required int covered, required int total}) {
    return _gradeFromBridge(
      bridge.keypointGrade(covered: covered, total: total),
    );
  }

  @override
  String mintTutorCard({
    required String front,
    required List<String> back,
    required int nowMs,
  }) {
    return _session.mintTutorCard(
      front: front,
      back: back,
      nowMs: BigInt.from(nowMs),
    );
  }

  @override
  void applyCardNote({required int line, required List<String> notes}) {
    _session.applyCardNote(line: line, notes: notes);
  }

  @override
  void applyExamPassed(int nowMs) {
    _session.applyExamPassed(nowMs: BigInt.from(nowMs));
  }

  @override
  int applyRemediation({required String cardsText, required int nowMs}) {
    return _session.applyRemediation(
      cardsText: cardsText,
      nowMs: BigInt.from(nowMs),
    );
  }
}

ReviewInput _inputFromBridge(bridge.Input input) => switch (input) {
  bridge.Input.type => ReviewInput.type,
  bridge.Input.draw => ReviewInput.draw,
};

ReviewStateModel _stateFromBridge(bridge.ReviewState state) {
  return ReviewStateModel(
    card: state.card == null ? null : _cardFromBridge(state.card!),
    mode: _modeFromBridge(state.mode),
    depth: _depthFromBridge(state.depth),
    input: _inputFromBridge(state.input),
    introducing: state.introducing,
    choices: state.choices,
    choiceRuns: state.choiceRuns == null
        ? null
        : [for (final runs in state.choiceRuns!) inlineRunsFromBridge(runs)],
    keypoints: state.keypoints,
    keypointRuns: state.keypointRuns == null
        ? null
        : [for (final runs in state.keypointRuns!) inlineRunsFromBridge(runs)],
    finished: state.finished,
    remaining: state.remaining,
    reviews: state.reviews,
    passed: state.passed,
    failed: state.failed,
    introduced: state.introduced,
    partial: state.partial,
    canRestart: state.canRestart,
    nextDueMs: state.nextDueMs?.toInt(),
    dueLeft: state.dueLeft,
    newLeft: state.newLeft,
    saveError: state.saveError,
    loadWarnings: state.loadWarnings,
  );
}

ReviewCardModel _cardFromBridge(bridge.CardView card) {
  return ReviewCardModel(
    front: card.front,
    frontRuns: inlineRunsFromBridge(card.frontRuns),
    frontUnits: card.frontUnits == null
        ? null
        : [for (final unit in card.frontUnits!) _noteFromBridge(unit)],
    context: card.context,
    contextLeads: card.contextLeads,
    contextRuns: [
      for (final runs in card.contextRuns) inlineRunsFromBridge(runs),
    ],
    contextUnits: [for (final unit in card.contextUnits) _noteFromBridge(unit)],
    back: card.back,
    backRuns: [for (final runs in card.backRuns) inlineRunsFromBridge(runs)],
    backUnits: [for (final unit in card.backUnits) _noteFromBridge(unit)],
    reshaped: card.reshaped,
    note: [for (final unit in card.note) _noteFromBridge(unit)],
    images: [for (final image in card.images) _imageFromBridge(image)],
    imagesBack: [for (final image in card.imagesBack) _imageFromBridge(image)],
  );
}

ReviewRegionModel _regionFromBridge(bridge.RegionView region) {
  return ReviewRegionModel(
    role: switch (region.role) {
      bridge.RegionRole.asked => ReviewRegionRole.asked,
      bridge.RegionRole.mask => ReviewRegionRole.mask,
      bridge.RegionRole.cover => ReviewRegionRole.cover,
    },
    revealOnAnswer: region.revealOnAnswer,
    x: region.x,
    y: region.y,
    width: region.width,
    height: region.height,
    unit: region.unit,
  );
}

ReviewImageModel _imageFromBridge(bridge.ImageView image) {
  return ReviewImageModel(
    src: image.src,
    alt: image.alt,
    regions: [for (final region in image.regions) _regionFromBridge(region)],
    crop: image.crop == null
        ? null
        : ReviewCropModel(
            x: image.crop!.x,
            y: image.crop!.y,
            width: image.crop!.width,
            height: image.crop!.height,
            unit: image.crop!.unit,
          ),
  );
}

ReviewNoteUnitModel _noteFromBridge(bridge.NoteUnit unit) {
  return switch (unit) {
    bridge.NoteUnit_Sentence(:final text, :final runs) => ReviewSentenceModel(
      text: text,
      runs: inlineRunsFromBridge(runs),
    ),
    bridge.NoteUnit_Code(:final lines) => ReviewCodeModel(lines),
    bridge.NoteUnit_Diagram(
      :final src,
      :final width,
      :final height,
      :final alt,
      :final regions,
      :final revealedAlt,
    ) =>
      ReviewDiagramModel(
        src: src,
        width: width,
        height: height,
        alt: alt,
        regions: [for (final region in regions) _regionFromBridge(region)],
        revealedAlt: revealedAlt,
      ),
    bridge.NoteUnit_Checklist(:final items) => ReviewChecklistModel([
      for (final item in items)
        ReviewChecklistItemModel(
          checked: item.checked,
          text: item.text,
          runs: inlineRunsFromBridge(item.runs),
        ),
    ]),
    bridge.NoteUnit_Table(:final aligns, :final header, :final rows) =>
      ReviewTableModel(
        aligns: [for (final align in aligns) _cellAlignFromBridge(align)],
        header: [for (final cell in header) inlineRunsFromBridge(cell)],
        rows: [
          for (final row in rows)
            [for (final cell in row) inlineRunsFromBridge(cell)],
        ],
      ),
  };
}

ReviewCellAlign _cellAlignFromBridge(bridge.CellAlign align) {
  return switch (align) {
    bridge.CellAlign.none => ReviewCellAlign.none,
    bridge.CellAlign.left => ReviewCellAlign.left,
    bridge.CellAlign.center => ReviewCellAlign.center,
    bridge.CellAlign.right => ReviewCellAlign.right,
  };
}

bridge.Depth? _depthToBridge(ReviewDepth? depth) {
  return switch (depth) {
    ReviewDepth.recognize => bridge.Depth.recognize,
    ReviewDepth.recall => bridge.Depth.recall,
    ReviewDepth.reconstruct => bridge.Depth.reconstruct,
    null => null,
  };
}

ReviewDepth _depthFromBridge(bridge.Depth depth) {
  return switch (depth) {
    bridge.Depth.recognize => ReviewDepth.recognize,
    bridge.Depth.recall => ReviewDepth.recall,
    bridge.Depth.reconstruct => ReviewDepth.reconstruct,
  };
}

ReviewMode _modeFromBridge(bridge.Mode mode) {
  return switch (mode) {
    bridge.Mode.flip => ReviewMode.flip,
    bridge.Mode.typing => ReviewMode.typing,
    bridge.Mode.typeLine => ReviewMode.typeLine,
    bridge.Mode.choice => ReviewMode.choice,
    bridge.Mode.lineByLine => ReviewMode.lineByLine,
    bridge.Mode.explain => ReviewMode.explain,
  };
}

bridge.Grade _gradeToBridge(ReviewGrade grade) {
  return switch (grade) {
    ReviewGrade.fail => bridge.Grade.fail,
    ReviewGrade.partial => bridge.Grade.partial,
    ReviewGrade.pass => bridge.Grade.pass,
  };
}

ReviewGrade _gradeFromBridge(bridge.Grade grade) {
  return switch (grade) {
    bridge.Grade.fail => ReviewGrade.fail,
    bridge.Grade.partial => ReviewGrade.partial,
    bridge.Grade.pass => ReviewGrade.pass,
  };
}
