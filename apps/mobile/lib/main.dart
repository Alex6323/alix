import 'package:flutter/material.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/picker_screen.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  final root = await prepare();
  runApp(AlixApp(root: root));
}

class AlixApp extends StatelessWidget {
  const AlixApp({super.key, required this.root});

  final String root;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'alix',
      theme: ThemeData(colorSchemeSeed: Colors.indigo),
      home: PickerScreen(root: root),
    );
  }
}
