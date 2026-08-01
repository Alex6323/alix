import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// A compact, deterministic projection of the rendered widget tree.
///
/// Framework layout nodes, visible text, keys, icons, fields, and action
/// enabledness remain. App-owned composition wrappers and Flutter lifecycle
/// machinery are transparent, so extracting an otherwise identical subtree
/// does not rewrite the baseline.
String normalizedWidgetTree(WidgetTester tester, Finder root) {
  final lines = <String>[];

  void walk(Element element, int depth, {bool insideChoiceOption = false}) {
    final choiceOption =
        insideChoiceOption || _isRandomizedChoiceOption(element.widget);
    final description = _describe(
      element.widget,
      insideChoiceOption: choiceOption,
    );
    final childDepth = description == null ? depth : depth + 1;
    if (description != null) {
      lines.add('${'  ' * depth}$description');
    }
    element.visitChildren(
      (child) => walk(child, childDepth, insideChoiceOption: choiceOption),
    );
  }

  walk(tester.element(root), 0);
  return '${lines.join('\n')}\n';
}

Future<void> expectWidgetTree(
  WidgetTester tester,
  String name, {
  required Finder root,
}) async {
  final actual = normalizedWidgetTree(tester, root);
  final rawDir = Platform.environment['RAW_WIDGET_TREE_DIR'];
  if (rawDir != null && rawDir.isNotEmpty) {
    final raw = File('$rawDir/$name.txt');
    raw.parent.createSync(recursive: true);
    raw.writeAsStringSync(tester.element(root).toStringDeep());
  }
  final snapshot = File('test/fixtures/widget_trees/$name.txt');
  if (Platform.environment['UPDATE_WIDGET_TREES'] == '1') {
    snapshot.parent.createSync(recursive: true);
    snapshot.writeAsStringSync(actual);
  }
  expect(
    snapshot.existsSync(),
    isTrue,
    reason:
        'missing ${snapshot.path}; run this suite once with '
        'UPDATE_WIDGET_TREES=1 to record the reviewed baseline',
  );
  expect(actual, snapshot.readAsStringSync(), reason: snapshot.path);
}

bool _isRandomizedChoiceOption(Widget widget) {
  final key = widget.key;
  return key is ValueKey<String> && key.value.startsWith('option-');
}

String? _describe(Widget widget, {required bool insideChoiceOption}) {
  final key = widget.key == null
      ? ''
      : ' key=${_clean('${widget.key}').replaceAll(RegExp(r'option-\d+'), 'option-<index>')}';
  if (widget is Text) {
    final value = insideChoiceOption
        ? '<choice-option>'
        : widget.data ?? widget.textSpan?.toPlainText() ?? '';
    return 'Text(${jsonEncode(_clean(value))})$key';
  }
  if (widget is TextField) {
    final decoration = widget.decoration;
    return '${widget.runtimeType}('
        'enabled=${widget.enabled ?? true}, '
        'label=${jsonEncode(_clean(decoration?.labelText ?? ''))}, '
        'hint=${jsonEncode(_clean(decoration?.hintText ?? ''))}, '
        'value=${jsonEncode(_clean(widget.controller?.text ?? ''))})$key';
  }
  if (widget is ButtonStyleButton) {
    return '${widget.runtimeType}(enabled=${widget.onPressed != null})$key';
  }
  if (widget is IconButton) {
    return 'IconButton(enabled=${widget.onPressed != null}, '
        'tooltip=${jsonEncode(_clean(widget.tooltip ?? ''))})$key';
  }
  if (widget is InkWell) {
    return 'InkWell(tap=${widget.onTap != null}, '
        'longPress=${widget.onLongPress != null})$key';
  }
  if (widget is ListTile) {
    return 'ListTile(enabled=${widget.enabled}, tap=${widget.onTap != null})$key';
  }
  if (widget is Icon) {
    final codePoint = widget.icon?.codePoint.toRadixString(16) ?? 'none';
    return 'Icon($codePoint)$key';
  }
  if (widget is Tooltip) {
    return 'Tooltip(${jsonEncode(_clean(widget.message ?? ''))})$key';
  }
  if (widget is Opacity) {
    final opacity = insideChoiceOption
        ? '<choice-state>'
        : widget.opacity.toStringAsFixed(2);
    return 'Opacity($opacity)$key';
  }
  if (widget is Padding) {
    return 'Padding(${_clean('${widget.padding}')})$key';
  }
  if (widget is SizedBox) {
    return 'SizedBox(width=${widget.width}, height=${widget.height})$key';
  }

  final type = '${widget.runtimeType}';
  const structural = {
    'Scaffold',
    'AppBar',
    'SafeArea',
    'Column',
    'Row',
    'Wrap',
    'Stack',
    'Center',
    'Align',
    'Expanded',
    'Flexible',
    'Spacer',
    'Container',
    'ConstrainedBox',
    'IntrinsicHeight',
    'SingleChildScrollView',
    'ListView',
    'Divider',
    'Material',
    'Card',
    'ClipRRect',
    'Image',
    'SvgPicture',
    'CustomPaint',
    'CircularProgressIndicator',
    'LinearProgressIndicator',
  };
  return structural.contains(type) ? '$type$key' : null;
}

String _clean(String value) {
  return value
      .replaceAll(RegExp(r'/[^\s]+/alix-[^/\s]+'), '<tmp>')
      .replaceAll(
        RegExp(r"Last written by '[^']+' \d+ min ago"),
        "Last written by '<device>' <age> ago",
      )
      .replaceAll(RegExp(r'GlobalKey#[0-9a-f]+'), 'GlobalKey#<id>')
      .replaceAll(RegExp(r'<tmp>/progress/.*'), '<save-error>')
      .replaceAll(RegExp(r'Next due in \d+ min\.'), 'Next due in <minutes>.')
      .replaceAll(RegExp(r'Next due in \d+ h\.'), 'Next due in <hours>.')
      .replaceAll(RegExp(r'Next due in \d+ days\.'), 'Next due in <days>.')
      .replaceAll(RegExp(r'\b\d+\.\d+\.\d+\b'), '<version>')
      .replaceAll(RegExp(r'\s+'), ' ')
      .trim();
}
