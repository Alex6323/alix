// The pairing sheet, split out of picker_screen.dart so a sub-screen (exam,
// review, tutor) can reopen it directly from a "Re-pair" SnackBarAction after
// a 401, without pulling in picker_screen.dart's own imports (notably
// src/rust/api/*: exam_screen.dart and tutor_sheet.dart deliberately never
// import the generated bridge, so their tests run without a Rust dylib; this
// file only reaches bootstrap.dart and server_client.dart, keeping that seam
// intact).
import 'dart:io';

import 'package:flutter/material.dart';

import 'package:alix_mobile/bootstrap.dart';
import 'package:alix_mobile/server_client.dart';

/// Opens the pairing sheet: paste the URL `alix --lan` prints, probe it, and
/// persist on success. The only pairing surface in the app; every caller
/// (the picker's own menu, and any screen's "Re-pair" SnackBarAction after a
/// 401) opens it through here. Pops with a status message on success (paired
/// or unpaired), or null if dismissed with nothing decided; a failed probe
/// shows one inline line inside the sheet itself, never a dialog.
Future<String?> showPairingSheet(
  BuildContext context, {
  required Directory support,
  required ServerClient Function(ServerConfig) buildClient,
}) {
  return showModalBottomSheet<String>(
    context: context,
    isScrollControlled: true,
    builder: (sheet) => _PairSheet(
      support: support,
      current: readServer(support),
      buildClient: buildClient,
    ),
  );
}

/// The pairing sheet's body: a paste field, a Pair button, an inline status
/// line, and, while paired, the current host:port with a ghost Unpair
/// button. Pops with a SnackBar message on success (paired or unpaired),
/// or stays open showing `_status` on any failure.
class _PairSheet extends StatefulWidget {
  const _PairSheet({
    required this.support,
    required this.current,
    required this.buildClient,
  });

  final Directory support;
  final ServerConfig? current;
  final ServerClient Function(ServerConfig) buildClient;

  @override
  State<_PairSheet> createState() => _PairSheetState();
}

class _PairSheetState extends State<_PairSheet> {
  final _controller = TextEditingController();
  String? _status;
  bool _busy = false;

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  Future<void> _pair() async {
    final parsed = parsePairingUrl(_controller.text);
    if (parsed == null) {
      setState(() => _status = 'that does not look like an alix pairing URL');
      return;
    }
    setState(() {
      _busy = true;
      _status = null;
    });
    final client = widget.buildClient(parsed);
    String? version;
    var refused = false;
    try {
      version = await client.version();
    } on PairingExpired {
      // alix answered and rejected the token: the pasted URL is stale (a
      // restarted server mints a fresh token). Say so distinctly; "no alix
      // answered" would send the user chasing the wrong problem.
      refused = true;
    } finally {
      client.close();
    }
    if (!mounted) return;
    if (refused) {
      setState(() {
        _busy = false;
        _status = 'alix answered but refused this token. '
            'Copy a fresh pairing URL from the server.';
      });
      return;
    }
    if (version == null) {
      setState(() {
        _busy = false;
        _status = 'no alix answered at ${parsed.host}:${parsed.port}';
      });
      return;
    }
    if (compareVersions(version, minServerVersion) < 0) {
      setState(() {
        _busy = false;
        _status = 'alix $version found, this app needs $minServerVersion or newer';
      });
      return;
    }
    await setServer(parsed, support: widget.support);
    if (!mounted) return;
    Navigator.of(context).pop('Paired with ${parsed.host}');
  }

  Future<void> _unpair() async {
    await setServer(null, support: widget.support);
    if (!mounted) return;
    Navigator.of(context).pop('Unpaired');
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final current = widget.current;
    return SafeArea(
      child: Padding(
        padding: EdgeInsets.fromLTRB(24, 24, 24, 24 + MediaQuery.of(context).viewInsets.bottom),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text('Pair with an alix server', style: theme.textTheme.titleMedium),
            const SizedBox(height: 8),
            if (current != null) ...[
              Text(
                'Paired with ${current.host}:${current.port}',
                style: theme.textTheme.bodySmall?.copyWith(
                  color: theme.colorScheme.onSurfaceVariant,
                  fontFamily: 'monospace',
                ),
              ),
              const SizedBox(height: 16),
            ],
            TextField(
              key: const ValueKey('pairing-url-field'),
              controller: _controller,
              decoration: const InputDecoration(
                labelText: 'Pairing URL',
                hintText: 'http://<ip>:<port>/?token=...',
              ),
              maxLines: 1,
            ),
            const SizedBox(height: 12),
            FilledButton(
              onPressed: _busy ? null : _pair,
              child: Text(_busy ? 'Pairing…' : 'Pair'),
            ),
            if (_status != null) ...[
              const SizedBox(height: 8),
              Text(
                _status!,
                style: theme.textTheme.bodySmall?.copyWith(color: theme.colorScheme.error),
              ),
            ],
            if (current != null) ...[
              const SizedBox(height: 8),
              TextButton(
                onPressed: _busy ? null : _unpair,
                child: const Text('Unpair'),
              ),
            ],
          ],
        ),
      ),
    );
  }
}
