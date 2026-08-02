import 'package:alix_mobile/src/rust/api/simple.dart' as bridge;

Future<void> stampSeededDeck(String path) => bridge.stampDeck(path: path);
