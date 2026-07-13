import 'package:flutter/material.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/review_screen.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  final prepared = await prepare();
  runApp(
    AlixApp(deckPath: prepared.deckPath, storeDir: prepared.storeDir),
  );
}

class AlixApp extends StatelessWidget {
  const AlixApp({super.key, required this.deckPath, required this.storeDir});

  final String deckPath;
  final String storeDir;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'alix',
      theme: ThemeData(colorSchemeSeed: Colors.indigo),
      home: ReviewScreen(deckPath: deckPath, storeDir: storeDir),
    );
  }
}
