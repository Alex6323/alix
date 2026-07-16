// The decks-root resolution order, driven with an injected support dir and
// env override so no platform channel is touched.
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/bootstrap.dart';

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

}
