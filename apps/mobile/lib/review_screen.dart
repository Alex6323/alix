import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge.dart'
    show AnyhowException;
import 'package:path_provider/path_provider.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/exam_screen.dart';
import 'package:alix_mobile/pairing_sheet.dart';
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/api/review.dart';
import 'package:alix_mobile/theme.dart';
import 'package:alix_mobile/tutor_sheet.dart';

const _mono = 'IBM Plex Mono';
const _sans = 'IBM Plex Sans';

/// The review screen: renders the core's ReviewState and feeds the learner's
/// actions back. All review logic lives in Rust; this widget switches on
/// `acquire` and `mode` and forwards taps. The surface mirrors the web
/// client (assets/web/review.html): a mono mode-tag, a bold question over a
/// faded divider, mode-specific answer bodies, a warm boxed note, and the
/// web's chip legend.
class ReviewScreen extends StatefulWidget {
  const ReviewScreen({
    super.key,
    required this.deckPath,
    required this.rootDir,
    required this.depth,
    this.device,
    this.supportDir,
    this.buildClient,
  });

  final String deckPath;
  final String rootDir;

  /// `null` lets the core resolve the deck's remembered depth, or its
  /// default when it has none (the picker's tap path).
  final Depth? depth;

  /// This install's label for the store's last-writer marker; also enables
  /// the "another device wrote this recently" banner.
  final String? device;

  /// The support dir the pairing config is read from; null uses the real
  /// app support dir. Tests inject a temp one.
  final Directory? supportDir;

  /// Builds the server-live probe's (and the tutor sheet's) client; null
  /// uses [HttpServerClient]. Tests inject a fake, following how
  /// PickerScreen injects its own probe client.
  final ServerClient Function(ServerConfig)? buildClient;

  @override
  State<ReviewScreen> createState() => _ReviewScreenState();
}

class _ReviewScreenState extends State<ReviewScreen> {
  late ReviewSession _session;
  late ReviewState _state;

  /// Why the session could not open (a trace deck, an unparseable file, an
  /// IO error). Rendered as a calm message with a way back; without this a
  /// failed open left `_state` uninitialized and the screen built white.
  String? _openError;
  bool _revealed = false;
  int _revealedLines = 0;
  ChoiceFeedback? _choice;
  CheckFeedback? _check;
  final List<TextEditingController> _typed = [];

  /// Explain: which keypoint rows are ticked (covered), and the optional
  /// pre-reveal attempt (client-only, never sent).
  final Set<int> _ticked = {};
  bool _attemptOpen = false;
  final TextEditingController _attempt = TextEditingController();

  /// Another device's recent write of this store, shown once per session
  /// open until dismissed.
  ForeignWriter? _foreign;

  /// Set once the pairing probe finds a reachable, current-enough desktop:
  /// gates the Ask chip. No status chrome for the negative cases (absent
  /// pairing, dead server, too old) per the UI-noise gate: the chip simply
  /// does not exist.
  bool _serverLive = false;
  ServerClient? _client;

  /// Resolved once `_probeServer` finishes its async lookup, regardless of
  /// whether pairing is present or live: cached so `_openExam` (only
  /// reachable once `_serverLive` is true, i.e. after this has settled) can
  /// hand its "Re-pair" action down without a fresh async support-dir
  /// resolution of its own. (`_openTutor` does not: TutorSheet's own 401
  /// SnackBar is unreachable while it stays open as a modal sheet on top of
  /// its own ROOT messenger, so wiring "Re-pair" there is deferred; see the
  /// task report.)
  Directory? _support;

  @override
  void initState() {
    super.initState();
    _open();
    if (_openError == null) _probeServer();
  }

  @override
  void dispose() {
    for (final c in _typed) {
      c.dispose();
    }
    _attempt.dispose();
    _client?.close();
    super.dispose();
  }

  /// Probes a configured pairing once per screen open: absence (no config,
  /// unreachable, refused, or an older server) leaves the Ask chip out
  /// entirely, no retry, no periodic re-probe. The one exception is a 401:
  /// the paired server is right there but rejects this app's token (a
  /// restarted desktop mints a fresh one), so it gets one SnackBar naming
  /// the fix; a merely dead server stays silent.
  Future<void> _probeServer() async {
    final support = widget.supportDir ?? await getApplicationSupportDirectory();
    _support = support;
    final config = readServer(support);
    if (config == null) return;
    final client = (widget.buildClient ?? HttpServerClient.new)(config);
    String? version;
    try {
      version = await client.version();
    } on PairingExpired {
      client.close();
      if (!mounted) return;
      // Unlike exam_screen.dart, this screen never pops itself here, so its
      // own context stays valid for as long as `mounted` holds; no
      // navigator capture needed.
      ScaffoldMessenger.of(context).showSnackBar(SnackBar(
        content: const Text('Pairing expired. Pair again from the deck list menu.'),
        action: SnackBarAction(
          label: 'Re-pair',
          onPressed: () {
            if (!mounted) return;
            showPairingSheet(
              context,
              support: support,
              buildClient: widget.buildClient ?? HttpServerClient.new,
            );
          },
        ),
      ));
      return;
    }
    final live = version != null && compareVersions(version, minServerVersion) >= 0;
    if (!live || !mounted) {
      client.close();
      return;
    }
    setState(() {
      _client = client;
      _serverLive = true;
    });
  }

  /// Opens the tutor sheet over [tutor], the current card's authored
  /// fields (never the masked [CardView] a cloze review renders). The
  /// sheet never touches the bridge itself; these two closures, to
  /// `mintTutorCard` and `applyCardNote`, are its only path back to it.
  /// `applyCardNote` targets [tutor]'s own deck-file line (`tutor.line`,
  /// a `BigInt` on the Dart side), captured by this closure the same way
  /// `mint` captures the session.
  void _openTutor(TutorCard tutor) {
    final client = _client;
    if (client == null) return;
    showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      builder: (_) => TutorSheet(
        card: TutorCardContext(
          subject: tutor.subject,
          front: tutor.front,
          back: tutor.back,
          at: tutor.at,
        ),
        client: client,
        mint: (front, back) async => _session.mintTutorCard(
          front: front,
          back: back,
          nowMs: BigInt.from(DateTime.now().millisecondsSinceEpoch),
        ),
        onNote: (notes) =>
            _session.applyCardNote(line: tutor.line.toInt(), notes: notes),
      ),
    );
  }

  /// The name the paired server's own catalog resolves this deck by
  /// (`resolve_row`, `src/serve/catalog.rs`, keyed off `picker::catalog`,
  /// `src/picker.rs`): a bare top-level file name, or `<workspace>/<file>`
  /// for a workspace member deck. The screen only holds device-absolute
  /// paths (`deckPath`, `rootDir`, both sourced from `DeckEntry`/
  /// `PickerScreen.root`, themselves device-absolute), never the
  /// server-relative key the picker keys rows by, so this derives it fresh:
  /// strip the root prefix, then rejoin the remaining path components with
  /// `/`. One level of drilling only, matching this app's own navigation
  /// (root -> loose deck, or root -> workspace folder -> member deck), so
  /// the result is exactly a bare name or a two-part qualified key, never
  /// deeper.
  String _deckName() {
    var rel = widget.deckPath;
    if (rel.startsWith(widget.rootDir)) {
      rel = rel.substring(widget.rootDir.length);
    }
    final parts = rel.split(RegExp(r'[\\/]+')).where((p) => p.isNotEmpty);
    return parts.join('/');
  }

  /// Pushes the exam screen over the current deck. The screen never touches
  /// the bridge itself; these two closures plus a nowMs provider are its
  /// only path back to it, mirroring `_openTutor`'s seam.
  void _openExam() {
    final client = _client;
    final support = _support;
    if (client == null || support == null) return;
    Navigator.of(context).push(MaterialPageRoute(
      builder: (_) => ExamScreen(
        deckName: _deckName(),
        client: client,
        support: support,
        buildClient: widget.buildClient ?? HttpServerClient.new,
        applyPassed: (nowMs) => _session.applyExamPassed(nowMs: nowMs),
        applyRemediation: (cardsText, nowMs) =>
            _session.applyRemediation(cardsText: cardsText, nowMs: nowMs),
        nowMs: () => BigInt.from(DateTime.now().millisecondsSinceEpoch),
      ),
    ));
  }

  /// (Re)opens the session; also the restart action on the done screen.
  void _open() {
    try {
      _session = ReviewSession.open(
        deckPath: widget.deckPath,
        rootDir: widget.rootDir,
        depth: widget.depth,
        device: widget.device,
      );
    } catch (e) {
      _openError = e is AnyhowException ? e.message : '$e';
      return;
    }
    _openError = null;
    _foreign = widget.device == null ? null : _session.foreignWriter();
    _resetCard(_session.state());
  }

  /// Installs the next position and clears all per-card interaction state.
  void _resetCard(ReviewState next) {
    _state = next;
    _revealed = false;
    _revealedLines = 0;
    _choice = null;
    _check = null;
    _ticked.clear();
    _attemptOpen = false;
    _attempt.clear();
    final lines = next.mode == Mode.typeLine ? (next.card?.back.length ?? 1) : 1;
    while (_typed.length < lines) {
      _typed.add(TextEditingController());
    }
    for (final c in _typed) {
      c.clear();
    }
  }

  void _apply(ReviewState next) => setState(() => _resetCard(next));
  void _grade(Grade grade) => _apply(_session.grade(grade: grade));

  bool get _hasChoices =>
      _state.choices != null && _state.choices!.isNotEmpty;
  bool get _isTyping =>
      _state.mode == Mode.typing || _state.mode == Mode.typeLine;
  bool get _isExplain =>
      _state.mode == Mode.explain &&
      !_state.acquire &&
      _state.keypoints != null;
  bool _lineDone(CardView card) =>
      _state.mode == Mode.lineByLine && _revealedLines >= card.back.length;

  /// The failed-open surface: the reason, plainly, and a way back. Loud
  /// beats blank: the message is the core's own error text.
  Widget _cantOpen(BuildContext context, String message) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    return Scaffold(
      appBar: AppBar(title: const AlixWordmark()),
      body: Center(
        child: Padding(
          padding: const EdgeInsets.all(32),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                'CAN\'T OPEN THIS DECK',
                style: TextStyle(
                  fontFamily: _mono,
                  fontSize: 12,
                  letterSpacing: 2,
                  color: tokens.faint,
                ),
              ),
              const SizedBox(height: 16),
              Text(
                message,
                textAlign: TextAlign.center,
                style: theme.textTheme.bodyMedium?.copyWith(color: tokens.dim),
              ),
              const SizedBox(height: 24),
              FilledButton(
                onPressed: () => Navigator.of(context).maybePop(),
                child: const Text('Back'),
              ),
            ],
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    if (_openError != null) {
      return _cantOpen(context, _openError ?? '');
    }
    final card = _state.card;
    return PopScope(
      // Leaving is immediate once the session is done; while cards are still
      // due, a back gesture or the AppBar back asks first, so a session isn't
      // abandoned by a stray swipe.
      canPop: _state.finished,
      onPopInvokedWithResult: (didPop, _) async {
        if (didPop || !mounted) return;
        final navigator = Navigator.of(context);
        final leave = await _confirmLeave(context);
        if (leave && navigator.mounted) navigator.pop();
      },
      child: Scaffold(
      appBar: AppBar(
        title: const AlixWordmark(),
        actions: [
          if (!_state.finished)
            Padding(
              padding: const EdgeInsets.only(right: 16),
              child: Center(
                child: Text(
                  '${_state.remaining} left',
                  style: TextStyle(
                    fontFamily: _mono,
                    fontSize: 13,
                    color: Theme.of(context).alix.dim,
                  ),
                ),
              ),
            ),
        ],
      ),
      body: SafeArea(
        child: Column(
          children: [
            _crumbStrip(context),
            if (_foreign case final foreign?) _foreignBanner(context, foreign),
            Expanded(
              child: card == null
                  ? _done(context)
                  : Column(
                      children: [
                        Expanded(
                          child: SingleChildScrollView(
                            padding: const EdgeInsets.fromLTRB(20, 8, 20, 8),
                            child: SizedBox(
                              width: double.infinity,
                              child: _face(context, card),
                            ),
                          ),
                        ),
                        _legend(context, card),
                      ],
                    ),
            ),
          ],
        ),
      ),
      ),
    );
  }

  /// Confirms abandoning a session that still has due cards. Returns true to
  /// leave, false to keep reviewing.
  Future<bool> _confirmLeave(BuildContext context) async {
    final n = _state.remaining;
    final leave = await showDialog<bool>(
      context: context,
      builder: (dialogContext) => AlertDialog(
        title: const Text('Leave the review?'),
        content: Text(
          '$n card${n == 1 ? '' : 's'} still due in this session.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(dialogContext).pop(false),
            child: const Text('Keep reviewing'),
          ),
          TextButton(
            onPressed: () => Navigator.of(dialogContext).pop(true),
            child: const Text('Leave'),
          ),
        ],
      ),
    );
    return leave ?? false;
  }

  // ── crumb strip (region breadcrumb) ──────────────────────────────────────

  /// The "where am I" region breadcrumb, mirroring the web's `.crumb-strip`
  /// (assets/web/review.html): shown only when this session is
  /// topology-ordered and the current card sits in a region (`crumb()`
  /// returns `null` otherwise, e.g. every plain fact deck). Hidden means
  /// hidden: no reserved space, no divider. Recomputed on every build so
  /// it tracks the session as it advances.
  Widget _crumbStrip(BuildContext context) {
    final crumb = _session.crumb(
      nowMs: BigInt.from(DateTime.now().millisecondsSinceEpoch),
    );
    // Mirrors the web's `.crumb-strip:empty` + `regions.length` guard: a
    // crumb with no regions renders nothing, not an empty strip.
    if (crumb == null || crumb.regions.isEmpty) return const SizedBox.shrink();
    return CrumbStrip(crumb: crumb);
  }

  // ── card face ──────────────────────────────────────────────────────────

  /// The mode-tag label, matching the web's modeLabel().
  String _modeLabel() {
    final prefix = _state.promotable ? 'remediation · ' : '';
    if (_state.acquire) return '${prefix}new';
    if (_hasChoices) return '${prefix}choice';
    return prefix +
        switch (_state.mode) {
          Mode.typeLine => 'typing · line',
          Mode.typing => 'typing',
          Mode.explain => 'explain',
          Mode.lineByLine => 'line',
          Mode.choice => 'choice',
          Mode.flip => 'flip',
        };
  }

  Widget _face(BuildContext context, CardView card) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    final answered =
        _revealed || _check != null || _choice != null || _lineDone(card);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        const SizedBox(height: 8),
        _modeTag(_modeLabel(), tokens),
        const SizedBox(height: 12),
        Text(
          card.front,
          textAlign: TextAlign.center,
          style: TextStyle(
            fontFamily: _sans,
            fontWeight: FontWeight.w600,
            fontSize: 23,
            height: 1.4,
            color: theme.colorScheme.onSurface,
          ),
        ),
        if (card.image != null) ...[
          const SizedBox(height: 12),
          Image.file(File(card.image!), height: 180),
        ],
        for (final line in card.context) ...[
          const SizedBox(height: 8),
          Text(
            line,
            textAlign: TextAlign.center,
            style: theme.textTheme.titleMedium?.copyWith(color: tokens.text),
          ),
        ],
        const SizedBox(height: 14),
        _divider(tokens),
        const SizedBox(height: 22),
        _body(context, card, tokens),
        if (answered && card.imageBack != null) ...[
          const SizedBox(height: 12),
          Image.file(File(card.imageBack!), height: 180),
        ],
        if (_state.acquire && !_hasChoices && !_revealed) ...[
          const SizedBox(height: 18),
          Text(
            'new card: try to recall it, then reveal.',
            textAlign: TextAlign.center,
            style: TextStyle(
              color: tokens.dim,
              fontSize: 13,
              fontStyle: FontStyle.italic,
            ),
          ),
        ],
        if (answered && card.note.isNotEmpty) _note(card, tokens),
      ],
    );
  }

  Widget _modeTag(String label, AlixTokens tokens) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 3),
      decoration: BoxDecoration(
        border: Border.all(color: tokens.line),
        borderRadius: BorderRadius.circular(6),
      ),
      child: Text(
        label.toUpperCase(),
        style: TextStyle(
          fontFamily: _mono,
          fontSize: 10.5,
          letterSpacing: 1.7,
          color: tokens.faint,
        ),
      ),
    );
  }

  /// A hairline that fades at both ends, 70% of the card width.
  Widget _divider(AlixTokens tokens) {
    return FractionallySizedBox(
      widthFactor: 0.7,
      child: Container(
        height: 1,
        decoration: BoxDecoration(
          gradient: LinearGradient(
            colors: [Colors.transparent, tokens.line, tokens.line, Colors.transparent],
            stops: const [0, 0.18, 0.82, 1],
          ),
        ),
      ),
    );
  }

  /// The mode-specific answer body.
  Widget _body(BuildContext context, CardView card, AlixTokens tokens) {
    if (_hasChoices) return _options(tokens);
    if (_state.mode == Mode.lineByLine && !_state.acquire) {
      // Line mode keeps its revealed lines tight (no stanza gap).
      return _revealLines(
          context, card.back.take(_revealedLines).toList(), tokens,
          stanza: false);
    }
    if (_isTyping && !_state.acquire) return _typing(context, card, tokens);
    if (_isExplain) return _explainBody(context, card, tokens);
    if (_revealed || _check != null) {
      return _revealLines(context, card.back, tokens);
    }
    return const SizedBox.shrink();
  }

  /// Revealed answer lines: monospace, neutral ink, centered. A plain
  /// multi-line flip answer reads as a stanza (a blank line between lines);
  /// line-mode and the explain answer stay tight.
  Widget _revealLines(
      BuildContext context, List<String> lines, AlixTokens tokens,
      {bool stanza = true}) {
    final style = TextStyle(
      fontFamily: _mono,
      fontWeight: FontWeight.w500,
      fontSize: 18,
      height: 1.5,
      color: Theme.of(context).colorScheme.onSurface,
    );
    final gap = stanza && lines.length > 1 ? 22.0 : 6.0;
    return Column(
      children: [
        for (final (i, line) in lines.indexed) ...[
          if (i > 0) SizedBox(height: gap),
          Text(line, textAlign: TextAlign.center, style: style),
        ],
      ],
    );
  }

  // ── multiple choice ──────────────────────────────────────────────────────

  Widget _options(AlixTokens tokens) {
    final options = _state.choices ?? const [];
    return Column(
      children: [
        for (final (i, opt) in options.indexed) ...[
          if (i > 0) const SizedBox(height: 10),
          _optionRow(i, opt, tokens),
        ],
      ],
    );
  }

  Widget _optionRow(int i, String opt, AlixTokens tokens) {
    final choice = _choice;
    final locked = choice != null;
    Color numColor = tokens.faint;
    Color textColor = tokens.text;
    Color borderColor = tokens.line;
    Color? fill = Colors.white.withValues(alpha: 0.03);
    double opacity = 1;
    if (locked) {
      final correct = BigInt.from(i) == choice.correct;
      final wrong = BigInt.from(i) == choice.chosen && !choice.passed;
      if (correct) {
        numColor = textColor = borderColor = tokens.good;
        fill = tokens.good.withValues(alpha: 0.12);
      } else if (wrong) {
        numColor = textColor = borderColor = tokens.again;
        fill = tokens.again.withValues(alpha: 0.13);
      } else {
        opacity = 0.45;
      }
    }
    final inner = Container(
      constraints: const BoxConstraints(minHeight: 52),
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 13),
      decoration: BoxDecoration(
        color: fill,
        border: Border.all(color: borderColor),
        borderRadius: BorderRadius.circular(12),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          Text('${i + 1}',
              style: TextStyle(fontFamily: _mono, fontSize: 13.5, color: numColor)),
          const SizedBox(width: 14),
          Expanded(
            child: Text(
              opt,
              style: TextStyle(
                  fontFamily: _mono, fontSize: 16, height: 1.35, color: textColor),
            ),
          ),
        ],
      ),
    );
    if (locked) {
      return Opacity(key: ValueKey('option-$i'), opacity: opacity, child: inner);
    }
    return Material(
      key: ValueKey('option-$i'),
      color: Colors.transparent,
      child: InkWell(
        borderRadius: BorderRadius.circular(12),
        onTap: () => setState(() => _choice = _session.choose(chosen: i)),
        child: inner,
      ),
    );
  }

  // ── typed answer ─────────────────────────────────────────────────────────

  Widget _typing(BuildContext context, CardView card, AlixTokens tokens) {
    // After a check the fields are replaced by the per-line evidence (the
    // web clears the inputs and renders only the result).
    if (_check != null) {
      return Column(
        children: [
          for (final r in _check!.results) _evidenceLine(r, tokens),
        ],
      );
    }
    final onSurface = Theme.of(context).colorScheme.onSurface;
    final fields = _state.mode == Mode.typeLine ? card.back.length : 1;
    OutlineInputBorder border(Color c) => OutlineInputBorder(
          borderRadius: BorderRadius.circular(12),
          borderSide: BorderSide(color: c),
        );
    return Column(
      children: [
        for (var i = 0; i < fields; i++) ...[
          if (i > 0) const SizedBox(height: 10),
          ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 520),
            child: TextField(
              controller: _typed[i],
              textAlign: TextAlign.center,
              style: TextStyle(fontFamily: _mono, fontSize: 17, color: onSurface),
              decoration: InputDecoration(
                filled: true,
                fillColor: Colors.white.withValues(alpha: 0.04),
                contentPadding:
                    const EdgeInsets.symmetric(horizontal: 16, vertical: 13),
                enabledBorder: border(tokens.line),
                focusedBorder: border(tokens.bolt),
                border: border(tokens.line),
              ),
            ),
          ),
        ],
      ],
    );
  }

  /// Per-line typed evidence: a passed line in green with a ✓; a miss recedes
  /// in dim red with a ✗ and the expected answer in green beneath it.
  Widget _evidenceLine(TypedResult r, AlixTokens tokens) {
    final input = r.input.isEmpty ? '(blank)' : r.input;
    if (r.passed) {
      return Padding(
        padding: const EdgeInsets.symmetric(vertical: 3),
        child: Text.rich(
          TextSpan(children: [
            TextSpan(
                text: input,
                style: TextStyle(
                    fontFamily: _mono,
                    fontWeight: FontWeight.w500,
                    fontSize: 18,
                    color: tokens.good)),
            TextSpan(
                text: '  ✓',
                style: TextStyle(
                    color: tokens.good, fontWeight: FontWeight.w700, fontSize: 18)),
          ]),
          textAlign: TextAlign.center,
        ),
      );
    }
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Column(
        children: [
          // The miss line (your input + the ✗) recedes at half opacity; the
          // expected answer below stays full green.
          Opacity(
            opacity: 0.5,
            child: Text.rich(
              TextSpan(children: [
                TextSpan(
                    text: input,
                    style: TextStyle(fontFamily: _mono, fontSize: 15, color: tokens.again)),
                TextSpan(
                    text: '  ✗',
                    style: TextStyle(
                        color: tokens.again,
                        fontWeight: FontWeight.w700,
                        fontSize: 15)),
              ]),
              textAlign: TextAlign.center,
            ),
          ),
          Text(
            r.expected,
            textAlign: TextAlign.center,
            style: TextStyle(
                fontFamily: _mono,
                fontWeight: FontWeight.w500,
                fontSize: 18,
                color: tokens.good),
          ),
        ],
      ),
    );
  }

  void _submitCheck(CardView card) => setState(() {
        final fields = _state.mode == Mode.typeLine ? card.back.length : 1;
        _check = _session.check(
          lines: [for (var i = 0; i < fields; i++) _typed[i].text],
        );
      });

  // ── explain keypoint checklist ───────────────────────────────────────────

  Widget _explainBody(BuildContext context, CardView card, AlixTokens tokens) {
    if (!_revealed) {
      // An optional client-only attempt: write it before revealing, or just
      // reveal. Never sent, never graded.
      if (!_attemptOpen) {
        return Align(
          child: TextButton(
            onPressed: () => setState(() => _attemptOpen = true),
            child: const Text('type your answer first'),
          ),
        );
      }
      return ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 520),
        child: TextField(
          controller: _attempt,
          minLines: 2,
          maxLines: 5,
          decoration: InputDecoration(
            filled: true,
            fillColor: Colors.black.withValues(alpha: 0.25),
            border: OutlineInputBorder(borderRadius: BorderRadius.circular(12)),
            hintText: 'your answer (stays on this device)',
          ),
        ),
      );
    }
    final points = _state.keypoints ?? const [];
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (_attempt.text.trim().isNotEmpty) ...[
          _explainLabel('your answer', tokens.dim),
          const SizedBox(height: 6),
          Text(
            _attempt.text.trim(),
            style: TextStyle(color: tokens.text, height: 1.4),
          ),
          const SizedBox(height: 16),
        ],
        _explainLabel('the answer', tokens.dim),
        const SizedBox(height: 6),
        _revealLines(context, card.back, tokens, stanza: false),
        const SizedBox(height: 16),
        _explainLabel('did your answer cover these?', tokens.good, small: true),
        const SizedBox(height: 6),
        for (final (i, pt) in points.indexed) _keypointRow(i, pt, tokens),
      ],
    );
  }

  Widget _explainLabel(String text, Color color, {bool small = false}) {
    return Text(
      text.toUpperCase(),
      style: TextStyle(
        fontFamily: _mono,
        fontSize: small ? 9.5 : 10.5,
        letterSpacing: 1.4,
        color: color,
      ),
    );
  }

  /// A keypoint row: tap to mark it covered (green ✓) or not (green ▸).
  Widget _keypointRow(int i, String pt, AlixTokens tokens) {
    final ticked = _ticked.contains(i);
    return InkWell(
      key: ValueKey('kp-$i'),
      borderRadius: BorderRadius.circular(7),
      onTap: () => setState(
          () => ticked ? _ticked.remove(i) : _ticked.add(i)),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 6, horizontal: 2),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
              width: 22,
              child: Text(ticked ? '✓' : '▸',
                  style: TextStyle(color: tokens.good, fontSize: 15, height: 1.45)),
            ),
            Expanded(
              child: Text(
                pt,
                style: TextStyle(
                  color: Theme.of(context).colorScheme.onSurface,
                  height: 1.45,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  // ── the note block ───────────────────────────────────────────────────────

  Widget _note(CardView card, AlixTokens tokens) {
    return Column(
      children: [
        const SizedBox(height: 18),
        _divider(tokens),
        const SizedBox(height: 14),
        Container(
          width: double.infinity,
          constraints: const BoxConstraints(maxWidth: 600),
          padding: const EdgeInsets.symmetric(horizontal: 15, vertical: 12),
          decoration: BoxDecoration(
            color: tokens.noteBorder.withValues(alpha: 0.12),
            border: Border.all(color: tokens.noteBorder.withValues(alpha: 0.24)),
            borderRadius: BorderRadius.circular(10),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              for (final (i, note) in card.note.indexed) ...[
                if (i > 0) const SizedBox(height: 10),
                switch (note) {
                  NoteUnit_Sentence(:final text) => Text(
                      text,
                      style: TextStyle(
                          color: tokens.noteInk, fontSize: 15, height: 1.4),
                    ),
                  NoteUnit_Code(:final lines) => Container(
                      width: double.infinity,
                      padding: const EdgeInsets.symmetric(
                          horizontal: 12, vertical: 10),
                      decoration: BoxDecoration(
                        color: Colors.black.withValues(alpha: 0.32),
                        borderRadius: BorderRadius.circular(8),
                      ),
                      child: Text(
                        lines.join('\n'),
                        style: TextStyle(
                            fontFamily: _mono,
                            fontSize: 13,
                            height: 1.45,
                            color: tokens.text),
                      ),
                    ),
                },
              ],
            ],
          ),
        ),
      ],
    );
  }

  // ── the footer legend (web chip parity) ──────────────────────────────────

  Widget _legend(BuildContext context, CardView card) {
    final chips = _legendChips(card);
    if (chips.isEmpty) {
      return SizedBox(height: 12 + MediaQuery.of(context).padding.bottom);
    }
    return Padding(
      padding: EdgeInsets.fromLTRB(
          12, 10, 12, 12 + MediaQuery.of(context).padding.bottom),
      child: Wrap(
        alignment: WrapAlignment.center,
        spacing: 10,
        runSpacing: 8,
        children: chips,
      ),
    );
  }

  /// Whether the learner has made an attempt on [card]: revealed it,
  /// picked an option, submitted a check, or walked all its lines. Mirrors
  /// the web client's attempt-first rule for the tutor: help arrives after
  /// you have tried, never instead of trying.
  bool _attempted(CardView card) {
    if (_hasChoices) return _choice != null;
    if (_state.acquire) return _revealed;
    if (_state.mode == Mode.lineByLine) return _lineDone(card);
    if (_isTyping) return _check != null;
    return _revealed;
  }

  /// The web's renderLegend() state matrix, mapped to chips, plus the Ask
  /// chip appended when a paired desktop answered the probe AND an attempt
  /// was made: gated on `session.tutorCard()` too, since a card can be
  /// showing with nothing authored to ground the tutor on (keyboard hints
  /// are dropped, touch only; Skip awaits a later milestone).
  List<Widget> _legendChips(CardView card) {
    final chips = [..._modeChips(card)];
    final tutor = _session.tutorCard();
    if (_serverLive && tutor != null && _attempted(card)) {
      chips.add(_chip('Ask', _ChipKind.quiet, () => _openTutor(tutor)));
    }
    return chips;
  }

  List<Widget> _modeChips(CardView card) {
    // Acquire: reveal, then acknowledge with Seen.
    if (_state.acquire) {
      if (_hasChoices) {
        return _choice == null
            ? const []
            : [_chip('Seen', _ChipKind.primary, () => _apply(_session.acquire()))];
      }
      return _revealed
          ? [_chip('Seen', _ChipKind.primary, () => _apply(_session.acquire()))]
          : [_chip('Reveal', _ChipKind.primary, () => setState(() => _revealed = true))];
    }
    // Recognize multiple choice.
    if (_hasChoices) {
      final choice = _choice;
      if (choice == null) return const [];
      return choice.passed
          ? [
              _chip('Next', _ChipKind.primary, () => _grade(Grade.pass)),
              _chip('I guessed', _ChipKind.quiet, () => _grade(Grade.fail)),
            ]
          : [_chip('Continue', _ChipKind.primary, () => _grade(Grade.fail))];
    }
    // Line-by-line reveal.
    if (_state.mode == Mode.lineByLine) {
      if (!_lineDone(card)) {
        return [
          _chip(_revealedLines == 0 ? 'Reveal' : 'Reveal next',
              _ChipKind.primary, () => setState(() => _revealedLines++)),
        ];
      }
      return _gradeTrio();
    }
    // Typed answer (the line variant checks one submission at a time).
    if (_isTyping) {
      if (_check == null) {
        final label = _state.mode == Mode.typeLine ? 'Check' : 'Submit';
        return [_chip(label, _ChipKind.primary, () => _submitCheck(card))];
      }
      return _gradeTrio();
    }
    // Explain checklist.
    if (_isExplain) {
      if (!_revealed) {
        return [_chip('Reveal', _ChipKind.primary, () => setState(() => _revealed = true))];
      }
      return [_verdictChip()];
    }
    // Flip / recognize-fallback: reveal, then grade.
    if (!_revealed) {
      return [_chip('Reveal', _ChipKind.primary, () => setState(() => _revealed = true))];
    }
    if (_state.depth == Depth.recognize) {
      return [
        _chip('Knew it', _ChipKind.passed, () => _grade(Grade.pass)),
        _chip('Not yet', _ChipKind.failed, () => _grade(Grade.fail)),
      ];
    }
    return _gradeTrio();
  }

  List<Widget> _gradeTrio() => [
        _chip('Missed it', _ChipKind.failed, () => _grade(Grade.fail)),
        _chip('Partly', _ChipKind.partly, () => _grade(Grade.partial)),
        _chip('Got it', _ChipKind.passed, () => _grade(Grade.pass)),
      ];

  /// The explain verdict chip: a filled chip showing the tick-tally's grade,
  /// committing it on tap (the web's Passed/Partly/Failed submit chip).
  Widget _verdictChip() {
    final tokens = Theme.of(context).alix;
    final grade = keypointGrade(
        covered: _ticked.length, total: (_state.keypoints ?? const []).length);
    final (label, color) = switch (grade) {
      Grade.fail => ('Failed', tokens.again),
      Grade.partial => ('Partly', tokens.warn),
      Grade.pass => ('Passed', tokens.good),
    };
    return _chip(label, _ChipKind.verdict, () => _grade(grade), verdictColor: color);
  }

  Widget _chip(String label, _ChipKind kind, VoidCallback? onTap,
      {Color? verdictColor}) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    Color? fill;
    Color borderColor = Colors.transparent;
    Color fg;
    FontWeight weight = FontWeight.w600;
    switch (kind) {
      case _ChipKind.base:
        borderColor = tokens.line;
        fg = tokens.text;
      case _ChipKind.primary:
        fill = theme.colorScheme.primary;
        borderColor = theme.colorScheme.primary;
        fg = theme.colorScheme.onPrimary;
      case _ChipKind.failed:
        fill = tokens.again.withValues(alpha: 0.12);
        borderColor = tokens.again.withValues(alpha: 0.42);
        fg = tokens.again;
      case _ChipKind.partly:
        fill = tokens.warn.withValues(alpha: 0.14);
        borderColor = tokens.warn.withValues(alpha: 0.42);
        fg = tokens.warn;
      case _ChipKind.passed:
        fill = tokens.good.withValues(alpha: 0.13);
        borderColor = tokens.good.withValues(alpha: 0.42);
        fg = tokens.good;
      case _ChipKind.quiet:
        fg = tokens.dim;
        weight = FontWeight.w400;
      case _ChipKind.verdict:
        fill = verdictColor;
        borderColor = verdictColor ?? Colors.transparent;
        fg = theme.colorScheme.surface;
    }
    return Material(
      color: fill ?? Colors.transparent,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(10),
        side: BorderSide(color: borderColor),
      ),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(10),
        // Content-width chips (like the web's inline-flex), so a grade trio
        // sits side by side. ~11px vertical padding gives the web's 42px.
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 15, vertical: 11),
          child: Text(
            label,
            style: TextStyle(
                fontFamily: _sans, fontWeight: weight, fontSize: 14, color: fg),
          ),
        ),
      ),
    );
  }

  // ── banners & done ───────────────────────────────────────────────────────

  /// The roaming-discipline warning: this store was just written elsewhere,
  /// so reviewing here would fork it.
  Widget _foreignBanner(BuildContext context, ForeignWriter foreign) {
    final theme = Theme.of(context);
    final minutes = (foreign.ageMs.toInt() / 60000).round();
    final age = minutes < 1 ? 'moments' : '$minutes min';
    return Container(
      margin: const EdgeInsets.fromLTRB(12, 8, 12, 0),
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
      decoration: BoxDecoration(
        color: theme.colorScheme.errorContainer,
        borderRadius: BorderRadius.circular(8),
      ),
      child: Row(
        children: [
          Expanded(
            child: Text(
              "Last written by '${foreign.device}' $age ago. "
              'Review on one device at a time.',
              style: theme.textTheme.bodySmall
                  ?.copyWith(color: theme.colorScheme.onErrorContainer),
            ),
          ),
          IconButton(
            icon: const Icon(Icons.close, size: 18),
            onPressed: () => setState(() => _foreign = null),
          ),
        ],
      ),
    );
  }

  /// The session-complete summary, mirroring the web's renderSummary().
  Widget _done(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = theme.alix;
    final reviews = _state.reviews;
    final acquired = _state.acquired;
    final acc = reviews > 0 ? '${(100 * _state.passed / reviews).round()}%' : '–';
    // A first pass over a fresh deck is acquire-only: reviews stay 0 while
    // every card was introduced. Say what actually happened.
    final headline = reviews > 0
        ? 'Nicely charged.'
        : acquired > 0
            ? 'New cards planted.'
            : 'Nothing due.';
    return SingleChildScrollView(
      padding: const EdgeInsets.all(24),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 12),
          Text(
            'SESSION COMPLETE',
            style: TextStyle(
                fontFamily: _mono,
                color: tokens.bolt,
                fontSize: 11,
                letterSpacing: 2.2),
          ),
          const SizedBox(height: 14),
          Text(
            headline,
            style: TextStyle(
                fontSize: 26,
                fontWeight: FontWeight.w600,
                color: theme.colorScheme.onSurface),
          ),
          const SizedBox(height: 18),
          if (acquired > 0) _summaryRow('introduced', '$acquired', tokens),
          _summaryRow('reviewed', '$reviews', tokens),
          if (reviews > 0) ...[
            _summaryRow('passed / failed',
                '${_state.passed} / ${_state.failed}', tokens),
            _summaryRow('accuracy', acc, tokens),
          ],
          if (!_state.canRestart) ...[
            const SizedBox(height: 18),
            Container(
              width: double.infinity,
              padding: const EdgeInsets.symmetric(horizontal: 15, vertical: 12),
              decoration: BoxDecoration(
                color: tokens.noteBorder.withValues(alpha: 0.12),
                border: Border.all(color: tokens.noteBorder.withValues(alpha: 0.24)),
                borderRadius: BorderRadius.circular(10),
              ),
              child: Text(
                'Nothing due right now, come back later.',
                style: TextStyle(color: tokens.noteInk, fontSize: 15),
              ),
            ),
          ],
          const SizedBox(height: 24),
          _chip(
            'New session',
            _state.canRestart ? _ChipKind.primary : _ChipKind.base,
            _state.canRestart ? () => setState(_open) : null,
          ),
          if (_serverLive && _session.deckHasExam()) ...[
            const SizedBox(height: 12),
            _chip('Take the exam', _ChipKind.quiet, _openExam),
          ],
        ],
      ),
    );
  }

  Widget _summaryRow(String label, String value, AlixTokens tokens) {
    return Container(
      padding: const EdgeInsets.symmetric(vertical: 9),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: tokens.line)),
      ),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Text(label, style: TextStyle(color: tokens.dim)),
          Text(
            value,
            style: TextStyle(
                fontFamily: _mono,
                fontWeight: FontWeight.w600,
                color: Theme.of(context).colorScheme.onSurface),
          ),
        ],
      ),
    );
  }
}

enum _ChipKind { base, primary, failed, partly, passed, quiet, verdict }

/// The region breadcrumb strip: each region's name over a thin per-card
/// heatmap bar (red = weak, green = strong; the web's `hsl(120*s, 62%,
/// (40+12*s)%)`), the current region emphasized (full ink, bold) and the
/// rest dimmed. A fixed-height, horizontally scrolling row, so a long path
/// or many regions never grows the header (clip, never wrap). Public, not
/// nested in [_ReviewScreenState], so a widget test can pump it directly
/// against a hand-built [CrumbState] without driving a live
/// topology-ordered session.
class CrumbStrip extends StatelessWidget {
  const CrumbStrip({super.key, required this.crumb});

  final CrumbState crumb;

  static const double height = 40;

  @override
  Widget build(BuildContext context) {
    final ink = Theme.of(context).colorScheme.onSurface;
    return SizedBox(
      height: height,
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        padding: const EdgeInsets.fromLTRB(20, 10, 20, 6),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            for (final (i, name) in crumb.regions.indexed) ...[
              if (i > 0) const SizedBox(width: 14),
              _region(
                name,
                i == crumb.current,
                i < crumb.cells.length ? crumb.cells[i] : const <double>[],
                ink,
              ),
            ],
          ],
        ),
      ),
    );
  }

  Widget _region(String name, bool current, Iterable<double> cells, Color ink) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        // A fixed height (rather than letting the text size the row) keeps
        // the strip's total height constant regardless of the active
        // font's line metrics, so a long name can never push the strip
        // taller and overflow its SizedBox.
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
          children: [
            for (final s in cells) _cell(s),
          ],
        ),
      ],
    );
  }

  /// One strength cell: the web's `hsl(120*s, 62%, (40+12*s)%)`, `s` clamped
  /// to 0..1 in case a stale cache ever hands back a stray float.
  Widget _cell(double s) {
    final clamped = s.clamp(0.0, 1.0);
    return Container(
      width: 5,
      height: 3,
      margin: const EdgeInsets.only(right: 1),
      decoration: BoxDecoration(
        color: HSLColor.fromAHSL(1, 120 * clamped, 0.62, (40 + 12 * clamped) / 100)
            .toColor(),
        borderRadius: BorderRadius.circular(1),
      ),
    );
  }
}
