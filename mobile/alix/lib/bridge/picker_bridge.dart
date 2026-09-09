import 'package:alix_mobile/bootstrap.dart' as bootstrap;
import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/picker_port.dart';
import 'package:alix_mobile/src/rust/api/generate.dart' as generate_bridge;
import 'package:alix_mobile/src/rust/api/listing.dart' as listing_bridge;
import 'package:alix_mobile/src/rust/api/review.dart' as review_bridge;
import 'package:alix_mobile/src/rust/api/simple.dart' as simple_bridge;

class PickerBridge implements PickerPort {
  const PickerBridge();

  @override
  PickerListing listRoot(String root, {required bool profile}) {
    final screen = listing_bridge.listRoot(root: root, profile: profile);
    return PickerListing(
      entries: [for (final entry in screen.entries) _entry(entry)],
      profile: _profile(screen.profile),
    );
  }

  @override
  PickerListing listMembers({
    required String root,
    required String dir,
    required bool profile,
  }) {
    final screen = listing_bridge.listMembers(
      root: root,
      dir: dir,
      profile: profile,
    );
    final deadline = screen.deadline;
    return PickerListing(
      entries: [for (final entry in screen.entries) _entry(entry)],
      deadline: deadline == null ? null : _deadline(deadline),
      profile: _profile(screen.profile),
    );
  }

  PickerProfile? _profile(listing_bridge.OpenProfile? profile) {
    if (profile == null) return null;
    return PickerProfile(
      libMs: profile.libMs.toInt(),
      counters: [
        ('candidates_classified', profile.candidatesClassified.toInt()),
        ('manifest_reads', profile.manifestReads.toInt()),
        ('decks_loaded', profile.decksLoaded.toInt()),
        ('prerequisite_loads', profile.prerequisiteLoads.toInt()),
        ('id_scans', profile.idScans.toInt()),
        ('diagram_geometry_reads', profile.diagramGeometryReads.toInt()),
        ('store_documents_read', profile.storeDocumentsRead.toInt()),
        ('augment_documents_read', profile.augmentDocumentsRead.toInt()),
        ('canonicalize_calls', profile.canonicalizeCalls.toInt()),
      ],
    );
  }

  @override
  void setWorkspaceDeadline({required String dir, required String? date}) {
    listing_bridge.setWorkspaceDeadline(dir: dir, date: date);
  }

  @override
  Future<void> addTutorialDeck(String root) => bootstrap.addTutorialDeck(root);

  @override
  String get coreVersion => simple_bridge.coreVersion();

  @override
  String applyGeneratedDeck({
    required String decksDir,
    required String filename,
    required String text,
  }) {
    return generate_bridge.applyGeneratedDeck(
      decksDir: decksDir,
      filename: filename,
      text: text,
    );
  }
}

PickerEntry _entry(listing_bridge.DeckEntry entry) {
  return PickerEntry(
    title: entry.title,
    path: entry.path,
    isWorkspace: entry.isWorkspace,
    due: entry.due,
    canRecognize: entry.canRecognize,
    isTrace: entry.isTrace,
    lastDepth: _depth(entry.lastDepth),
    mastered: entry.mastered,
    examDue: entry.examDue,
    hasExam: entry.hasExam,
    locked: entry.locked,
    progressError: entry.progressError,
    icon: entry.icon,
    indent: entry.indent,
    tree: entry.tree,
    deadline: entry.deadline == null ? null : _deadline(entry.deadline!),
  );
}

PickerDeadline _deadline(listing_bridge.Deadline deadline) {
  return PickerDeadline(
    date: deadline.date,
    daysLeft: deadline.daysLeft,
    ready: deadline.ready,
    total: deadline.total,
  );
}

PickerDepth _depth(review_bridge.Depth depth) {
  return switch (depth) {
    review_bridge.Depth.recognize => PickerDepth.recognize,
    review_bridge.Depth.recall => PickerDepth.recall,
    review_bridge.Depth.reconstruct => PickerDepth.reconstruct,
  };
}
