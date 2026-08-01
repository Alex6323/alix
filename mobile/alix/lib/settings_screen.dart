import 'package:flutter/material.dart';

/// The Settings page: pushed up from the bottom by the picker's hamburger, a
/// back arrow (top-left) returns it. Two accented rows first (Support,
/// Connected devices), a dim divider, then the plain settings. Each row opens
/// its own sheet/dialog over this page; the picker below owns all the state,
/// so this screen holds none of its own.
class SettingsScreen extends StatelessWidget {
  const SettingsScreen({
    super.key,
    required this.onSupport,
    required this.onConnectedDevices,
    required this.onDecksFolder,
    required this.onTheme,
    required this.onAbout,
    this.onGenerate,
  });

  final VoidCallback onSupport;
  final VoidCallback onConnectedDevices;
  final VoidCallback onDecksFolder;
  final VoidCallback onTheme;
  final VoidCallback onAbout;

  /// Non-null only when a paired desktop is reachable (generate needs it); the
  /// row is omitted otherwise.
  final VoidCallback? onGenerate;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    // The content-changing rows return to the picker first (Settings slides
    // back down), then act, so you see the result on the list; Decks folder
    // MUST, since choosing one remounts the picker (its state, which these
    // callbacks are bound to, is disposed). The config rows below just open a
    // sheet over Settings and leave you here (Signal-style dwell).
    void popThen(VoidCallback action) {
      Navigator.of(context).pop();
      action();
    }

    return Scaffold(
      appBar: AppBar(title: const Text('Settings')),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.symmetric(vertical: 8),
          children: [
            ListTile(
              leading: Icon(Icons.favorite, color: theme.colorScheme.primary),
              title: const Text('Support alix'),
              onTap: onSupport,
            ),
            ListTile(
              leading: const Icon(Icons.devices),
              title: const Text('Connected devices'),
              onTap: onConnectedDevices,
            ),
            Divider(
              height: 24,
              thickness: 1,
              indent: 16,
              endIndent: 16,
              color: theme.dividerColor,
            ),
            ListTile(
              leading: const Icon(Icons.folder_outlined),
              title: const Text('Decks folder'),
              onTap: () => popThen(onDecksFolder),
            ),
            if (onGenerate != null)
              ListTile(
                leading: const Icon(Icons.auto_awesome_outlined),
                title: const Text('Generate deck'),
                onTap: () => popThen(onGenerate!),
              ),
            ListTile(
              leading: const Icon(Icons.palette_outlined),
              title: const Text('Theme'),
              onTap: onTheme,
            ),
            ListTile(
              leading: const Icon(Icons.info_outline),
              title: const Text('About'),
              onTap: onAbout,
            ),
          ],
        ),
      ),
    );
  }
}
