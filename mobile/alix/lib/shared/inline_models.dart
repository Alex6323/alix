class InlineRunModel {
  const InlineRunModel({
    required this.text,
    required this.bold,
    required this.italic,
    required this.code,
    this.math,
  });

  final String text;
  final bool bold;
  final bool italic;
  final bool code;
  final InlineMathModel? math;

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        other is InlineRunModel &&
            text == other.text &&
            bold == other.bold &&
            italic == other.italic &&
            code == other.code &&
            math == other.math;
  }

  @override
  int get hashCode => Object.hash(text, bold, italic, code, math);
}

class InlineMathModel {
  const InlineMathModel({required this.display, this.svg, this.error});

  final bool display;
  final String? svg;
  final String? error;

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        other is InlineMathModel &&
            display == other.display &&
            svg == other.svg &&
            error == other.error;
  }

  @override
  int get hashCode => Object.hash(display, svg, error);
}
