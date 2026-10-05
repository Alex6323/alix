import 'dart:io';

import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/bridge/walk_bridge.dart';
import 'package:alix_mobile/pairing_sheet.dart';
import 'package:alix_mobile/review/review_card.dart' show SectionSheet;
import 'package:alix_mobile/review/review_models.dart';
import 'package:alix_mobile/review/review_view.dart' show ReviewCantOpenView;
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/tutor_conversation.dart';
import 'package:alix_mobile/tutor_sheet.dart';
import 'package:alix_mobile/walk/walk_controller.dart';
import 'package:alix_mobile/walk/walk_port.dart';
import 'package:alix_mobile/walk/walk_view.dart';

class WalkScreen extends StatefulWidget {
  const WalkScreen({
    super.key,
    required this.deckPath,
    required this.rootDir,
    this.device,
    this.supportDir,
    this.buildClient,
    this.factory = const WalkBridgeFactory(),
  });

  final String deckPath;
  final String rootDir;
  final String? device;
  final Directory? supportDir;
  final ServerClient Function(ServerConfig)? buildClient;
  final WalkPortFactory factory;

  @override
  State<WalkScreen> createState() => _WalkScreenState();
}

class _WalkScreenState extends State<WalkScreen> {
  late final WalkController _controller;
  final TextEditingController _attempt = TextEditingController();

  ServerClient? _client;
  String? _autoOpenedSection;

  TutorConversation? _tutor;
  final List<TutorConversation> _retiredTutors = [];
  final _tutorSlot = TutorSlot();
  bool _tutorSheetOpen = false;

  @override
  void initState() {
    super.initState();
    _controller = WalkController(
      factory: widget.factory,
      deckPath: widget.deckPath,
      rootDir: widget.rootDir,
      device: widget.device,
    );
    if (_controller.openError == null) _probeServer();
  }

  @override
  void dispose() {
    _attempt.dispose();
    _controller.dispose();
    _tutor?.dispose();
    for (final retired in _retiredTutors) {
      retired.dispose();
    }
    _tutorSlot.dispose();
    _client?.close();
    super.dispose();
  }

  Future<void> _probeServer() async {
    final support = widget.supportDir ?? await getApplicationSupportDirectory();
    final config = readActivePairing(support);
    if (config == null) return;
    final client = (widget.buildClient ?? HttpServerClient.new)(config);
    ServerVersion? probe;
    try {
      probe = await client.version();
    } on PairingExpired {
      client.close();
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: const Text(
            'Pairing expired. Pair again from Settings → Connected devices.',
          ),
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
        ),
      );
      return;
    }
    final live =
        probe != null && compareVersions(probe.version, minServerVersion) >= 0;
    if (!live || !mounted) {
      client.close();
      return;
    }
    _client = client;
    _controller.setServerLive(true);
  }

  void _openTutor(ReviewTutorCardModel tutor) {
    final client = _client;
    if (client == null) return;
    final conversation = _tutorFor(tutor, client);
    _tutorSheetOpen = true;
    showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      builder: (_) => TutorSheet(conversation: conversation),
    ).whenComplete(() => _tutorSheetOpen = false);
  }

  TutorConversation _tutorFor(ReviewTutorCardModel tutor, ServerClient client) {
    final current = _tutor;
    if (current != null && current.card.cardId == tutor.id) return current;
    if (current != null) _retireTutor(current);
    return _tutor = TutorConversation(
      card: TutorCardContext(
        deckId: tutor.deckId,
        cardId: tutor.id,
        subject: tutor.subject,
        front: tutor.front,
        back: tutor.back,
        at: tutor.at,
      ),
      client: client,
      slot: _tutorSlot,
      mint: (front, back) async =>
          _controller.mintTutorCard(front: front, back: back),
      onNote: (notes) => _controller.applyCardNote(id: tutor.id, notes: notes),
      onMessage: _tutorMessage,
    );
  }

  void _tutorMessage(String text) {
    if (_tutorSheetOpen || !mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(text)));
  }

  void _retireTutor(TutorConversation conversation) {
    if (!conversation.busy) {
      conversation.dispose();
      return;
    }
    _retiredTutors.add(conversation);
    void settled() {
      if (conversation.busy) return;
      conversation.removeListener(settled);
      _retiredTutors.remove(conversation);
      conversation.dispose();
    }

    conversation.addListener(settled);
  }

  void _openSection(ReviewCardModel card) {
    showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      builder: (_) => SectionSheet(card: card),
    );
  }

  void _autoOpenSection(bool sectionFirst, ReviewCardModel? card) {
    if (card == null || !sectionFirst || !card.hasSection) return;
    final key = card.section.join('\n');
    if (_autoOpenedSection == key) return;
    _autoOpenedSection = key;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _openSection(card);
    });
  }

  void _nextWalk() {
    _autoOpenedSection = null;
    _controller.nextWalk();
  }

  @override
  Widget build(BuildContext context) {
    if (_controller.openError case final error?) {
      return ReviewCantOpenView(message: error);
    }
    return ListenableBuilder(
      listenable: _controller,
      builder: (context, _) {
        final state = _controller.state;
        _autoOpenSection(state.sectionFirst, state.card);
        return WalkView(
          state: state,
          choice: _controller.choice,
          multiChoice: _controller.multiChoice,
          multiSelected: _controller.multiSelected,
          sketch: _controller.sketch,
          onSketchBegin: _controller.sketchBegin,
          onSketchExtend: _controller.sketchExtend,
          onSketchEnd: _controller.sketchEnd,
          onSketchTool: _controller.selectSketchTool,
          onSketchUndo: _controller.sketchUndo,
          onSketchClear: _controller.sketchClear,
          attemptController: _attempt,
          typedControllers: const [],
          serverLive: _controller.serverLive,
          tutorCard: state.card == null ? null : _controller.tutorCard,
          onChoose: _controller.choose,
          onToggleChoice: _controller.toggleChoice,
          onSubmitChoices: _controller.submitChoices,
          onReveal: _controller.reveal,
          onNext: _controller.next,
          onNextWalk: _nextWalk,
          onOpenTutor: _openTutor,
          onOpenSection: _openSection,
        );
      },
    );
  }
}
