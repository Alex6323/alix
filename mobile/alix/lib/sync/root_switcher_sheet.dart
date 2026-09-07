import 'package:flutter/material.dart';

import 'package:alix_mobile/server_client.dart' show ServerConfig;

/// Lists every paired desktop, the active one marked; tapping a different
/// one pops with it (null on dismiss or tapping the already-active row).
/// The caller persists the switch (`setActiveRoot`) and remounts the
/// picker on the returned root.
Future<ServerConfig?> showRootSwitcherSheet(
  BuildContext context, {
  required List<ServerConfig> pairings,
  required String? activeRootId,
}) {
  return showModalBottomSheet<ServerConfig>(
    context: context,
    builder: (_) =>
        _RootSwitcherSheet(pairings: pairings, activeRootId: activeRootId),
  );
}

class _RootSwitcherSheet extends StatelessWidget {
  const _RootSwitcherSheet({required this.pairings, required this.activeRootId});

  final List<ServerConfig> pairings;
  final String? activeRootId;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 8),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 8, 16, 8),
              child: Text('Paired desktop', style: theme.textTheme.titleMedium),
            ),
            for (final pairing in pairings)
              ListTile(
                leading: Icon(
                  pairing.rootId == activeRootId
                      ? Icons.check_circle
                      : Icons.dns_outlined,
                  color: pairing.rootId == activeRootId
                      ? theme.colorScheme.primary
                      : null,
                ),
                title: Text(
                  '${pairing.host}:${pairing.port}',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
                onTap: pairing.rootId == activeRootId
                    ? null
                    : () => Navigator.of(context).pop(pairing),
              ),
          ],
        ),
      ),
    );
  }
}
