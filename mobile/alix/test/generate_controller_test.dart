import 'package:flutter_test/flutter_test.dart';

import 'package:alix_mobile/picker/generate_controller.dart';
import 'package:alix_mobile/server_client.dart';

import 'support/fake_server_client.dart';

void main() {
  test('invalid URLs fail locally without starting the desktop', () async {
    final client = FakeServerClient();
    final controller = GenerateController(client: client);
    var notifications = 0;
    controller.addListener(() => notifications++);

    await controller.submit(url: 'file:///notes', guidance: 'anything');

    expect(client.generateStartCalled, isFalse);
    expect(
      controller.message,
      'alix can only generate from an http:// or https:// URL.',
    );
    expect(controller.busy, isFalse);
    expect(notifications, 1);

    controller.dispose();
    expect(client.generateCloseCalls, 1);
    expect(client.closed, isTrue);
  });

  test(
    'polling publishes progress and hands a completed deck to the route',
    () async {
      final client = FakeServerClient(
        generateGetReplies: const [
          RemoteGenerate(phase: 'generating', elapsed: 2),
          RemoteGenerate(
            phase: 'done',
            deck: '## q\na\n',
            filename: 'generated.md',
            cards: 1,
          ),
        ],
      );
      final scheduler = _FakePollScheduler();
      final controller = GenerateController(
        client: client,
        pollInterval: const Duration(milliseconds: 400),
        schedulePoll: scheduler.schedule,
      );

      await controller.submit(
        url: ' https://example.org ',
        guidance: ' focus on basics ',
      );

      expect(client.generateStartedUrl, 'https://example.org');
      expect(client.generateStartedGuidance, 'focus on basics');
      expect(controller.busy, isTrue);
      expect(controller.elapsed, 2);
      expect(scheduler.delay, const Duration(milliseconds: 400));

      await scheduler.fire();
      expect(controller.completed?.filename, 'generated.md');

      final completed = controller.handoff();
      expect(completed.deck, contains('## q'));
      controller.dispose();
      expect(client.generateCloseCalls, 0);
      expect(client.closed, isFalse);

      await client.generateClose();
      client.close();
    },
  );

  test('a refused start returns to the retryable idle state', () async {
    final client = FakeServerClient(generateStartReply: false);
    final controller = GenerateController(client: client);

    await controller.submit(url: 'https://example.org', guidance: '');

    expect(controller.busy, isFalse);
    expect(controller.message, 'The desktop refused to generate this deck.');
    expect(controller.elapsed, isNull);
    expect(controller.completed, isNull);
    controller.dispose();
  });
}

class _FakePollScheduler {
  Duration? delay;
  Future<void> Function()? callback;
  _FakePollTimer? timer;

  GeneratePollTimer schedule(
    Duration nextDelay,
    Future<void> Function() nextCallback,
  ) {
    delay = nextDelay;
    callback = nextCallback;
    timer = _FakePollTimer();
    return timer!;
  }

  Future<void> fire() async {
    final next = callback;
    if (next == null) throw StateError('no poll scheduled');
    await next();
  }
}

class _FakePollTimer implements GeneratePollTimer {
  bool cancelled = false;

  @override
  void cancel() => cancelled = true;
}
