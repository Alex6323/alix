import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';

import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/review_port.dart';
import 'package:alix_mobile/review/sketch.dart';

class ReviewController extends ChangeNotifier {
  factory ReviewController({
    required ReviewPortFactory factory,
    required String deckPath,
    required String rootDir,
    ReviewDepth? depth,
    String? device,
  }) {
    return ReviewController._(factory, deckPath, rootDir, depth, device);
  }

  ReviewController._(
    this._factory,
    this._deckPath,
    this._rootDir,
    this._depth,
    this._device,
  ) {
    _open();
  }

  final ReviewPortFactory _factory;
  final String _deckPath;
  final String _rootDir;
  final ReviewDepth? _depth;
  final String? _device;

  ReviewPort? _port;
  ReviewStateModel? _state;
  String? _openError;
  bool _revealed = false;
  int _revealedLines = 0;
  ReviewChoiceFeedbackModel? _choice;
  ReviewMultiChoiceFeedbackModel? _multiChoice;
  final Set<int> _multiSelected = {};
  ReviewCheckFeedbackModel? _checkFeedback;
  List<ReviewTypedResultModel> _typelineChecked = const [];
  final Set<int> _tickedKeypoints = {};
  final Sketch _sketch = Sketch();
  bool _attemptOpen = false;
  ReviewForeignWriterModel? _foreignWriter;
  bool _serverLive = false;

  ReviewStateModel get state {
    final state = _state;
    if (state == null) throw StateError('review session is not open');
    return state;
  }

  String? get openError => _openError;
  bool get revealed => _revealed;
  int get revealedLines => _revealedLines;
  ReviewChoiceFeedbackModel? get choice => _choice;
  ReviewMultiChoiceFeedbackModel? get multiChoice => _multiChoice;
  Set<int> get multiSelected => Set.unmodifiable(_multiSelected);
  ReviewCheckFeedbackModel? get checkFeedback => _checkFeedback;

  /// TypeLine's accepted prefix: the gradeable steps already checked, which
  /// the next check resubmits so the server always pairs by true position.
  List<ReviewTypedResultModel> get typelineChecked =>
      List.unmodifiable(_typelineChecked);
  Set<int> get tickedKeypoints => Set.unmodifiable(_tickedKeypoints);
  Sketch get sketch => _sketch;
  bool get isDrawing => state.input == ReviewInput.draw;
  bool get attemptOpen => _attemptOpen;
  ReviewForeignWriterModel? get foreignWriter => _foreignWriter;
  bool get serverLive => _serverLive;
  ReviewTutorCardModel? get tutorCard => _requirePort().tutorCard;
  bool get deckHasExam => _requirePort().deckHasExam;

  bool get hasChoices => state.choices?.isNotEmpty ?? false;
  bool get isTyping =>
      state.mode == ReviewMode.typing || state.mode == ReviewMode.typeLine;
  bool get isExplain =>
      state.mode == ReviewMode.explain &&
      !state.introducing &&
      state.keypoints != null;

  bool lineDone(ReviewCardModel card) {
    return state.mode == ReviewMode.lineByLine &&
        _revealedLines >= card.answerSteps.length;
  }

  void setServerLive(bool live) {
    if (_serverLive == live) return;
    _serverLive = live;
    notifyListeners();
  }

  void install(ReviewStateModel next) {
    _install(next);
    notifyListeners();
  }

  void choose(int chosen) {
    _choice = _requirePort().choose(chosen);
    notifyListeners();
  }

  void toggleChoice(int index) {
    if (!_multiSelected.remove(index)) _multiSelected.add(index);
    notifyListeners();
  }

  void submitChoices() {
    _multiChoice = _requirePort().chooseMulti(_multiSelected.toList());
    notifyListeners();
  }

  void check(List<String> lines) {
    if (state.mode == ReviewMode.typeLine) {
      // One gradeable step at a time. The accepted prefix is resubmitted with
      // the new line so the server pairs by true position, and its reply IS
      // the whole result set, so the last one doubles as the closing feedback.
      final sent = [
        for (final result in _typelineChecked) result.input,
        ...lines,
      ];
      final feedback = _requirePort().check(sent);
      if (feedback != null) {
        _typelineChecked = feedback.results;
        final owed = state.card?.gradeableSteps.length ?? 0;
        if (_typelineChecked.length >= owed) _checkFeedback = feedback;
      }
    } else {
      _checkFeedback = _requirePort().check(lines);
    }
    notifyListeners();
  }

  void openAttempt() {
    _attemptOpen = true;
    notifyListeners();
  }

  void toggleKeypoint(int index) {
    if (!_tickedKeypoints.remove(index)) _tickedKeypoints.add(index);
    notifyListeners();
  }

  void reveal() {
    _revealed = true;
    notifyListeners();
  }

  void selectSketchTool(SketchTool tool) {
    _sketch.selectTool(tool);
    notifyListeners();
  }

  void sketchBegin(Offset point, PointerDeviceKind kind) {
    _sketch.begin(point, kind);
    notifyListeners();
  }

  void sketchExtend(Offset point) {
    _sketch.extend(point);
    notifyListeners();
  }

  void sketchEnd() {
    _sketch.end();
    notifyListeners();
  }

  void sketchUndo() {
    _sketch.undo();
    notifyListeners();
  }

  void sketchClear() {
    _sketch.clear();
    notifyListeners();
  }

  void revealNextLine() {
    _revealedLines++;
    notifyListeners();
  }

  void dismissForeignWriter() {
    _foreignWriter = null;
    notifyListeners();
  }

  void restart() {
    _open();
    notifyListeners();
  }

  void introduce() => install(_requirePort().introduce());

  void grade(ReviewGrade grade) => install(_requirePort().grade(grade));

  ReviewGrade get verdictGrade => _requirePort().keypointGrade(
    covered: _tickedKeypoints.length,
    total: state.keypoints?.length ?? 0,
  );

  ReviewCrumbModel? crumb(int nowMs) => _requirePort().crumb(nowMs);

  String mintTutorCard({
    required String front,
    required List<String> back,
    required int nowMs,
  }) {
    return _requirePort().mintTutorCard(front: front, back: back, nowMs: nowMs);
  }

  void applyCardNote({required String id, required List<String> notes}) {
    _requirePort().applyCardNote(id: id, notes: notes);
  }

  void applyExamPassed(int nowMs) => _requirePort().applyExamPassed(nowMs);

  int applyRemediation({required String cardsText, required int nowMs}) {
    return _requirePort().applyRemediation(cardsText: cardsText, nowMs: nowMs);
  }

  ReviewPort _requirePort() {
    final port = _port;
    if (port == null) throw StateError('review session is not open');
    return port;
  }

  void _open() {
    try {
      final port = _factory.open(
        deckPath: _deckPath,
        rootDir: _rootDir,
        depth: _depth,
        device: _device,
      );
      _port = port;
      _openError = null;
      _foreignWriter = _device == null ? null : port.foreignWriter;
      _install(port.state);
    } on ReviewOpenFailure catch (error) {
      _port = null;
      _state = null;
      _openError = error.message;
    }
  }

  void _install(ReviewStateModel next) {
    _state = next;
    _revealed = false;
    _revealedLines = 0;
    _choice = null;
    _multiChoice = null;
    _multiSelected.clear();
    _checkFeedback = null;
    _typelineChecked = const [];
    _tickedKeypoints.clear();
    _sketch.reset();
    _attemptOpen = false;
  }
}
