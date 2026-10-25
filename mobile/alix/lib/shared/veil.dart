// The hidden look of a revealed answer, the same law as the web's veilRuns:
// each word keeps its first character, and its last past two characters;
// the rest is drawn transparent in place, so every word keeps its width.
import 'package:flutter/widgets.dart';

class VeilRun {
  const VeilRun(this.text, {required this.kept});

  final String text;
  final bool kept;
}

List<VeilRun> veilRuns(String text) {
  final runs = <VeilRun>[];
  void push(String chunk, bool kept) {
    if (chunk.isEmpty) return;
    if (runs.isNotEmpty && runs.last.kept == kept) {
      runs[runs.length - 1] = VeilRun(runs.last.text + chunk, kept: kept);
    } else {
      runs.add(VeilRun(chunk, kept: kept));
    }
  }

  for (final part in text.split(RegExp(r'(?=\s)|(?<=\s)'))) {
    if (part.isEmpty) continue;
    if (part.trim().isEmpty) {
      push(part, true);
      continue;
    }
    final chars = part.characters.toList();
    if (chars.length <= 2) {
      push(chars.first, true);
      push(chars.skip(1).join(), false);
      continue;
    }
    push(chars.first, true);
    push(chars.sublist(1, chars.length - 1).join(), false);
    push(chars.last, true);
  }
  return runs;
}

List<TextSpan> veilSpans(String text, TextStyle style) => [
  for (final run in veilRuns(text))
    TextSpan(
      text: run.text,
      style: run.kept ? style : style.copyWith(color: const Color(0x00000000)),
    ),
];

/// Plain text that veils under a [VeilScope], a [Text] otherwise.
class VeilText extends StatelessWidget {
  const VeilText(
    this.text, {
    super.key,
    required this.style,
    this.textAlign = TextAlign.start,
  });

  final String text;
  final TextStyle style;
  final TextAlign textAlign;

  @override
  Widget build(BuildContext context) {
    if (!VeilScope.of(context)) {
      return Text(text, textAlign: textAlign, style: style);
    }
    return Text.rich(
      TextSpan(children: veilSpans(text, style)),
      textAlign: textAlign,
      style: style,
    );
  }
}

/// Whether the text below is drawn veiled. Read by the inline run and unit
/// widgets, so one flag at the answer region hides every word under it.
class VeilScope extends InheritedWidget {
  const VeilScope({super.key, required this.veiled, required super.child});

  final bool veiled;

  static bool of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<VeilScope>()?.veiled ?? false;

  @override
  bool updateShouldNotify(VeilScope oldWidget) => veiled != oldWidget.veiled;
}

/// Something that is not text (an image, a diagram, display math): under a
/// veil it is invisible and keeps its size, so nothing around it moves.
class VeilInvisible extends StatelessWidget {
  const VeilInvisible({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Visibility(
      visible: !VeilScope.of(context),
      maintainSize: true,
      maintainState: true,
      maintainAnimation: true,
      child: child,
    );
  }
}
