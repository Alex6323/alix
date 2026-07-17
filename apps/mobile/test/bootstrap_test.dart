// The decks-root resolution order, driven with an injected support dir and
// env override so no platform channel is touched.
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/server_client.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  Directory temp(String prefix) {
    final dir = Directory.systemTemp.createTempSync(prefix);
    addTearDown(() {
      if (dir.existsSync()) {
        dir.deleteSync(recursive: true);
      }
    });
    return dir;
  }

  test('a fresh install lands on app storage, seeded, with a minted device',
      () async {
    final support = temp('alix-support-');
    final prepared = await prepare(support: support, env: '');
    expect(prepared.root, '${support.path}/decks');
    expect(File('${support.path}/decks/basics.txt').existsSync(), isTrue,
        reason: 'samples seed the fresh dir');
    expect(prepared.sharedDir, isNull);
    expect(prepared.staleDecksDir, isNull);
    expect(prepared.device, matches(RegExp(r'^phone-[0-9a-f]{4}$')));

    final again = await prepare(support: support, env: '');
    expect(again.device, prepared.device, reason: 'the label is minted once');
  });

  test('a fresh install seeds the tutorial deck', () async {
    final support = temp('alix-support-');
    await prepare(support: support, env: '');
    final tutorial = File('${support.path}/decks/tutorial.txt');
    expect(tutorial.existsSync(), isTrue);
    expect(tutorial.readAsStringSync(), contains('The alix tutorial'));
  });

  test('a deleted tutorial stays deleted on the next launch', () async {
    final support = temp('alix-support-');
    await prepare(support: support, env: '');
    final tutorial = File('${support.path}/decks/tutorial.txt');
    tutorial.deleteSync();
    await prepare(support: support, env: '');
    expect(tutorial.existsSync(), isFalse,
        reason: 'deleting the tutorial is the graduation; it must not return');
  });

  test('the env var wins over a configured shared folder', () async {
    final support = temp('alix-support-');
    final shared = temp('alix-shared-');
    await setDecksDir(shared.path, support: support);
    final prepared = await prepare(support: support, env: '/tmp/env-decks');
    expect(prepared.root, '/tmp/env-decks');
  });

  test('a listable shared folder is the root; a stale one falls back, kept',
      () async {
    final support = temp('alix-support-');
    final shared = temp('alix-shared-');
    await setDecksDir(shared.path, support: support);
    final live = await prepare(support: support, env: '');
    expect(live.root, shared.path);
    expect(live.sharedDir, shared.path);
    expect(live.staleDecksDir, isNull);

    shared.deleteSync(recursive: true);
    final fallen = await prepare(support: support, env: '');
    expect(fallen.root, '${support.path}/decks');
    expect(fallen.staleDecksDir, shared.path);

    // The setting survived the stale launch: restoring the folder heals it.
    shared.createSync(recursive: true);
    expect((await prepare(support: support, env: '')).root, shared.path);
  });

  test('a revoked storage grant falls back even though the dir still lists',
      () async {
    // On Android, revoking All Files Access does NOT make the dir
    // unlistable (FUSE filters it to empty), so prepare must trust the
    // grant query over the filesystem probe.
    final support = temp('alix-support-');
    final shared = temp('alix-shared-');
    await setDecksDir(shared.path, support: support);

    final revoked = await prepare(
      support: support,
      env: '',
      hasStorageAccess: () async => false,
    );
    expect(revoked.root, '${support.path}/decks');
    expect(revoked.staleDecksDir, shared.path);

    final granted = await prepare(
      support: support,
      env: '',
      hasStorageAccess: () async => true,
    );
    expect(granted.root, shared.path);
  });

  test('reverting to app storage clears the setting', () async {
    final support = temp('alix-support-');
    final shared = temp('alix-shared-');
    await setDecksDir(shared.path, support: support);
    await setDecksDir(null, support: support);
    final prepared = await prepare(support: support, env: '');
    expect(prepared.root, '${support.path}/decks');
    expect(prepared.sharedDir, isNull);
  });

  test('malformed settings read as empty instead of crashing the launch',
      () async {
    final support = temp('alix-support-');
    File('${support.path}/settings.json').writeAsStringSync('{not json');
    final prepared = await prepare(support: support, env: '');
    expect(prepared.root, '${support.path}/decks');
  });

  group('readServer / setServer', () {
    test('a paired server round-trips through set and read', () async {
      final support = temp('alix-support-');
      const config = ServerConfig(host: '192.168.1.5', port: 7777, token: 'abc123');
      await setServer(config, support: support);
      expect(readServer(support), config);
    });

    test('an absent server key reads as null', () async {
      final support = temp('alix-support-');
      expect(readServer(support), isNull);
    });

    test('a malformed server value reads as null, never throws', () async {
      final support = temp('alix-support-');

      File('${support.path}/settings.json').writeAsStringSync(jsonEncode({'server': 'not a map'}));
      expect(readServer(support), isNull);

      File('${support.path}/settings.json').writeAsStringSync(jsonEncode({
        'server': {'host': '1.2.3.4', 'port': 'eight', 'token': 'abc'},
      }));
      expect(readServer(support), isNull);
    });

    test('setServer(null) removes the key', () async {
      final support = temp('alix-support-');
      const config = ServerConfig(host: '192.168.1.5', port: 7777, token: 'abc123');
      await setServer(config, support: support);
      await setServer(null, support: support);
      expect(readServer(support), isNull);
      final raw = jsonDecode(File('${support.path}/settings.json').readAsStringSync()) as Map;
      expect(raw.containsKey('server'), isFalse);
    });
  });
}
