import 'package:alix_mobile/picker/picker_models.dart';

abstract interface class PickerPort {
  PickerListing listRoot(String root, {required bool profile});

  PickerListing listMembers({
    required String root,
    required String dir,
    required bool profile,
  });

  void setWorkspaceDeadline({required String dir, required String? date});

  Future<void> addTutorialDeck(String root);

  String get coreVersion;

  String applyGeneratedDeck({
    required String decksDir,
    required String filename,
    required String text,
  });
}
