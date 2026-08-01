import 'package:flutter/material.dart';

import 'package:alix_mobile/picker/generate_controller.dart';

class GenerateSheet extends StatefulWidget {
  const GenerateSheet({super.key, required this.controller});

  final GenerateController controller;

  @override
  State<GenerateSheet> createState() => _GenerateSheetState();
}

class _GenerateSheetState extends State<GenerateSheet> {
  final _urlController = TextEditingController();
  final _guidanceController = TextEditingController();
  bool _popping = false;

  @override
  void initState() {
    super.initState();
    widget.controller.addListener(_completeIfReady);
  }

  @override
  void dispose() {
    widget.controller.removeListener(_completeIfReady);
    widget.controller.dispose();
    _urlController.dispose();
    _guidanceController.dispose();
    super.dispose();
  }

  void _completeIfReady() {
    if (_popping || !mounted || widget.controller.completed == null) return;
    _popping = true;
    Navigator.of(context).pop(widget.controller.handoff());
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: widget.controller,
      builder: (context, _) {
        final theme = Theme.of(context);
        return SafeArea(
          child: Padding(
            padding: EdgeInsets.fromLTRB(
              24,
              24,
              24,
              24 + MediaQuery.of(context).viewInsets.bottom,
            ),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Text('Generate deck', style: theme.textTheme.titleMedium),
                const SizedBox(height: 8),
                if (widget.controller.busy)
                  Text(
                    widget.controller.elapsed != null
                        ? 'The desktop is working… '
                              '${widget.controller.elapsed}s'
                        : 'The desktop is working…',
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  )
                else ...[
                  TextField(
                    key: const ValueKey('generate-url-field'),
                    controller: _urlController,
                    decoration: const InputDecoration(
                      labelText: 'URL',
                      hintText: 'https://...',
                    ),
                    maxLines: 1,
                  ),
                  const SizedBox(height: 12),
                  TextField(
                    key: const ValueKey('generate-guidance-field'),
                    controller: _guidanceController,
                    decoration: const InputDecoration(
                      labelText: 'Guidance (optional)',
                    ),
                    maxLines: 1,
                  ),
                  const SizedBox(height: 12),
                  FilledButton(
                    onPressed: () => widget.controller.submit(
                      url: _urlController.text,
                      guidance: _guidanceController.text,
                    ),
                    child: const Text('Generate'),
                  ),
                  if (widget.controller.message != null) ...[
                    const SizedBox(height: 8),
                    Text(
                      widget.controller.message!,
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.error,
                      ),
                    ),
                  ],
                ],
              ],
            ),
          ),
        );
      },
    );
  }
}
