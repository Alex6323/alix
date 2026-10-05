import 'package:alix_mobile/picker/picker_models.dart';

abstract interface class PickerPort {
  Future<PickerListing> listRoot(String root, {required bool profile});

  Future<PickerListing> listMembers({
    required String root,
    required String dir,
    required bool profile,
  });

  Future<Map<String, PickerStrip>> deckStrips({
    required String root,
    required List<String> decks,
  });

  Future<List<PickerSearchHit>> listSearchable(List<String> roots);

  void setWorkspaceDeadline({required String dir, required String? date});

  Future<void> addTutorialDeck(String root);

  String get coreVersion;

  String applyGeneratedDeck({
    required String decksDir,
    required String filename,
    required String text,
  });
}
