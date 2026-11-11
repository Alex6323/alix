import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/picker/picker_models.dart';
import 'package:alix_mobile/picker/picker_view.dart';
import 'package:alix_mobile/picker/picker_widgets.dart';
import 'package:alix_mobile/theme.dart';

PickerEntry _entry(String title) => PickerEntry(
  title: title,
  path: '/desktop/$title.md',
  isWorkspace: false,
  due: true,
  canRecognize: true,
  isTrace: false,
  mastered: false,
  examDue: false,
  examable: false,
  locked: false,
  progressError: false,
  indent: 0,
  tree: '',
);

void main() {
  testWidgets('the empty-folder hint shows only while unpaired, at the root', (tester) async {
    for (final (label, paired, isRoot, shows) in [
      ('unpaired root', false, true, true),
      ('paired root', true, true, false),
      ('paired folder', true, false, true),
    ]) {
      await tester.pumpWidget(
        MaterialApp(
          theme: alixDark(),
          home: Scaffold(
            body: PickerView(
              entries: const [],
              deadline: null,
              isRoot: isRoot,
              isMasteredView: false,
              leading: const SizedBox(width: 56),
              onOpenEntry: (_) {},
              onLongPressEntry: (_) {},
              onOpenMastered: (_) {},
              onAddTutorial: () {},
              pairedLabel: paired ? 'iota' : null,
              pairedEntries: paired ? [_entry('Rust')] : const [],
            ),
          ),
        ),
      );
      expect(find.byType(PickerEmptyHint), shows ? findsOneWidget : findsNothing,
          reason: '$label: the empty-folder hint');
    }
  });
}
