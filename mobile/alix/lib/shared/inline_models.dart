class InlineRunModel {
  const InlineRunModel({
    required this.text,
    required this.bold,
    required this.italic,
    this.strike = false,
    required this.code,
    this.link = false,
    this.sub = false,
    this.sup = false,
    this.ins = false,
    this.math,
  });

  final String text;
  final bool bold;
  final bool italic;
  final bool strike;
  final bool code;
  final bool link;
  final bool sub;
  final bool sup;
  final bool ins;
  final InlineMathModel? math;

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        other is InlineRunModel &&
            text == other.text &&
            bold == other.bold &&
            italic == other.italic &&
            strike == other.strike &&
            code == other.code &&
            link == other.link &&
            sub == other.sub &&
            sup == other.sup &&
            ins == other.ins &&
            math == other.math;
  }

  @override
  int get hashCode =>
      Object.hash(text, bold, italic, strike, code, link, sub, sup, ins, math);
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
