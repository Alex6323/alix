import 'package:alix_mobile/picker/picker_models.dart';

abstract interface class PickerPort {
  List<PickerEntry> listRoot(String root);

  List<PickerEntry> listMembers({required String root, required String dir});

  List<String> syncConflicts(String root);

  PickerDeadline? workspaceDeadline({
    required String root,
    required String dir,
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
