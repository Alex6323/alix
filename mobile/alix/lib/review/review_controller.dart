import 'package:flutter/foundation.dart';

import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/review_port.dart';

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
  ReviewCheckFeedbackModel? _checkFeedback;
  final Set<int> _tickedKeypoints = {};
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
  ReviewCheckFeedbackModel? get checkFeedback => _checkFeedback;
  Set<int> get tickedKeypoints => Set.unmodifiable(_tickedKeypoints);
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
      !state.acquire &&
      state.keypoints != null;

  bool lineDone(ReviewCardModel card) {
    return state.mode == ReviewMode.lineByLine &&
        _revealedLines >= card.back.length;
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

  void check(List<String> lines) {
    _checkFeedback = _requirePort().check(lines);
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

  void acquire() => install(_requirePort().acquire());

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

  void applyCardNote({required int line, required List<String> notes}) {
    _requirePort().applyCardNote(line: line, notes: notes);
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
    _checkFeedback = null;
    _tickedKeypoints.clear();
    _attemptOpen = false;
  }
}
