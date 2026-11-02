// The decks-root resolution order, driven with an injected support dir and
// env override so no platform channel is touched.
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/bridge/sync_bridge.dart' as sync_bridge;
import 'package:alix_mobile/server_client.dart';
import 'package:alix_mobile/src/rust/frb_generated.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(RustLib.init);

  Directory temp(String prefix) {
    final dir = Directory.systemTemp.createTempSync(prefix);
    addTearDown(() {
      if (dir.existsSync()) {
        dir.deleteSync(recursive: true);
      }
    });
    return dir;
  }

  test('a fresh install lands on an empty app storage folder with a minted '
      'device', () async {
    final support = temp('alix-support-');
    final prepared = await prepare(support: support, env: '');
    expect(prepared.root, '${support.path}/decks');
    expect(Directory('${support.path}/decks').listSync(), isEmpty,
        reason: 'nothing is seeded: the tutorial opens from the menu');
    expect(prepared.device, matches(RegExp(r'^phone-[0-9a-f]{4}$')));

    final again = await prepare(support: support, env: '');
    expect(again.device, prepared.device, reason: 'the label is minted once');
  });

  test('the env var wins over app storage', () async {
    final support = temp('alix-support-');
    final prepared = await prepare(support: support, env: '/tmp/env-decks');
    expect(prepared.root, '/tmp/env-decks');
  });

  test('malformed settings read as empty instead of crashing the launch',
      () async {
    final support = temp('alix-support-');
    File('${support.path}/settings.json').writeAsStringSync('{not json');
    final prepared = await prepare(support: support, env: '');
    expect(prepared.root, '${support.path}/decks');
  });

  group('readPairings / readActivePairing / savePairing / setActiveRoot / removePairing', () {
    const configA = ServerConfig(host: '192.168.1.5', port: 7777, token: 'abc123', rootId: 'root-a');
    const configB = ServerConfig(host: '192.168.1.9', port: 7778, token: 'def456', rootId: 'root-b');

    test('a saved pairing round-trips through readPairings and becomes active', () async {
      final support = temp('alix-support-');
      await savePairing(configA, support: support);
      expect(readPairings(support), [configA]);
      expect(readActivePairing(support), configA);
    });

    test('no pairings and no active_root read as empty / null', () async {
      final support = temp('alix-support-');
      expect(readPairings(support), isEmpty);
      expect(readActivePairing(support), isNull);
    });

    test('saving a second, different rootId appends rather than replaces', () async {
      final support = temp('alix-support-');
      await savePairing(configA, support: support);
      await savePairing(configB, support: support);
      expect(readPairings(support), containsAll([configA, configB]));
      expect(readPairings(support), hasLength(2));
      expect(readActivePairing(support), configB, reason: 'the most recently saved pairing is active');
    });

    test('saving the same rootId again replaces the stored pairing, at most one per rootId', () async {
      final support = temp('alix-support-');
      await savePairing(configA, support: support);
      const updated = ServerConfig(host: '10.0.0.1', port: 9999, token: 'newtok', rootId: 'root-a');
      await savePairing(updated, support: support);
      expect(readPairings(support), [updated]);
      expect(readActivePairing(support), updated);
    });

    test('setActiveRoot switches which saved pairing readActivePairing returns', () async {
      final support = temp('alix-support-');
      await savePairing(configA, support: support);
      await savePairing(configB, support: support);
      await setActiveRoot(configA.rootId, support: support);
      expect(readActivePairing(support), configA);
    });

    test('removePairing drops the entry and clears active_root when it pointed there', () async {
      final support = temp('alix-support-');
      await savePairing(configA, support: support);
      await removePairing(configA.rootId, support: support);
      expect(readPairings(support), isEmpty);
      expect(readActivePairing(support), isNull);
    });

    test('removePairing leaves active_root alone when it names a different pairing', () async {
      final support = temp('alix-support-');
      await savePairing(configA, support: support);
      await savePairing(configB, support: support);
      await removePairing(configA.rootId, support: support);
      expect(readPairings(support), [configB]);
      expect(readActivePairing(support), configB);
    });

    test('a malformed pairings entry is skipped, valid ones still read', () async {
      final support = temp('alix-support-');
      await savePairing(configA, support: support);
      final settings = readSettings(support);
      final pairings = List<dynamic>.from(settings['pairings'] as List)
        ..add('not a map')
        ..add({'host': '1.2.3.4', 'port': 'eight', 'token': 'abc', 'root_id': 'root-c'});
      settings['pairings'] = pairings;
      File('${support.path}/settings.json').writeAsStringSync(jsonEncode(settings));
      expect(readPairings(support), [configA]);
    });

    test('an entirely malformed pairings value reads as empty, never throws', () async {
      final support = temp('alix-support-');
      File('${support.path}/settings.json').writeAsStringSync(jsonEncode({'pairings': 'not a list'}));
      expect(readPairings(support), isEmpty);
    });
  });

  group('readTheme / setTheme', () {
    test('a theme choice round-trips through set and read', () async {
      final support = temp('alix-support-');
      await setTheme('solarized-light', support: support);
      expect(readTheme(support), 'solarized-light');
    });

    test('an absent theme key reads as null', () async {
      final support = temp('alix-support-');
      expect(readTheme(support), isNull);
    });

    test('a malformed theme value reads as null, never throws', () async {
      final support = temp('alix-support-');
      File('${support.path}/settings.json')
          .writeAsStringSync(jsonEncode({'theme': 42}));
      expect(readTheme(support), isNull);
    });

    test('prepare surfaces the saved theme id', () async {
      final support = temp('alix-support-');
      await setTheme('dracula', support: support);
      final prepared = await prepare(support: support, env: '');
      expect(prepared.themeId, 'dracula');
    });

    test('prepare surfaces a null themeId when no theme was ever saved',
        () async {
      final support = temp('alix-support-');
      final prepared = await prepare(support: support, env: '');
      expect(prepared.themeId, isNull);
    });

    test('setTheme(null) removes only the theme key; other keys survive',
        () async {
      final support = temp('alix-support-');
      const config = ServerConfig(host: '192.168.1.5', port: 7777, token: 'abc123', rootId: 'root-a');
      await savePairing(config, support: support);
      await setTheme('solarized-light', support: support);

      await setTheme(null, support: support);

      expect(readTheme(support), isNull);
      final raw = jsonDecode(File('${support.path}/settings.json').readAsStringSync()) as Map;
      expect(raw.containsKey('theme'), isFalse);
      expect(readActivePairing(support), config);
    });
  });

  test(
    'a pairing does not replace the app-private decks root: it stays the '
    "phone's own, created even with a pairing active",
    () async {
      final support = temp('alix-support-');
      await savePairing(
        const ServerConfig(
          host: '127.0.0.1',
          port: 7777,
          token: 'abc',
          rootId: 'root-test0000000000000000000000',
        ),
        support: support,
      );

      final prepared = await prepareWithPairing(support: support, env: '');

      expect(prepared.root, '${support.path}/decks');
      expect(
        Directory('${support.path}/decks').existsSync(),
        isTrue,
        reason: 'pairing must never skip creating the phone\'s own folder',
      );
    },
  );

  test(
    'a paired recovery failure at open still returns a usable Prepared, '
    "root untouched",
    () async {
      final support = temp('alix-support-');
      await savePairing(
        const ServerConfig(
          host: '127.0.0.1',
          port: 7777,
          token: 'abc',
          rootId: 'root-brken000000000000000000000',
        ),
        support: support,
      );
      final pairedDir = sync_bridge.pairedRootDirFor(
        support: support.path,
        rootId: 'root-brken000000000000000000000',
      );
      // A regular file sits where the paired root must be a directory:
      // pairedRecoverFor's create_dir_all/rollback cannot succeed over it,
      // reproducing a real recovery failure rather than a mocked one.
      Directory(pairedDir).parent.createSync(recursive: true);
      File(pairedDir).writeAsStringSync('not a directory');

      final prepared = await prepareWithPairing(support: support, env: '');

      expect(prepared.root, '${support.path}/decks');
    },
  );
}
