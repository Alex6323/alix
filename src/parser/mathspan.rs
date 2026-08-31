//! The structural-unit policy for span matches inside math source (ADR
//! 0040): a masked range must be a complete structural unit of its formula,
//! so replacing it with a boxed blank can never leave dangling structure.
//! A whole-formula match (compared after trimming outer whitespace) is
//! always such a unit and bypasses the token rules; the renderer re-parse
//! backstop in the binding code still checks it.

use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Violation {
    EndpointInsideControlSequence(String),
    StructuralToken(char),
    Comment,
    RowBreak,
    GroupSplit,
    ControlSequence(String),
    IncompleteScript(char),
    IncompleteCommandApplication(String),
    NotLearnerVisible(String),
    MutableNamespace(String),
}

impl Violation {
    pub(super) fn message(&self) -> String {
        match self {
            Violation::EndpointInsideControlSequence(name) => {
                format!("a match edge cuts into the control sequence `{name}`")
            }
            Violation::StructuralToken(token) => {
                format!("the match contains the structural token `{token}`")
            }
            Violation::Comment => "the match lies inside a TeX comment".to_string(),
            Violation::RowBreak => r"the match contains a `\\` row break".to_string(),
            Violation::GroupSplit => "the match splits a brace group".to_string(),
            Violation::ControlSequence(name) => format!(
                "the match contains the control sequence `{name}`, which is not a blankable symbol"
            ),
            Violation::IncompleteScript(token) => {
                format!("the match contains `{token}` without its complete base and script")
            }
            Violation::IncompleteCommandApplication(name) => format!(
                "the match cuts the application of `{name}`: an argument group continues past it"
            ),
            Violation::NotLearnerVisible(name) => {
                format!("the match lies inside the argument of `{name}`, which renders nothing")
            }
            Violation::MutableNamespace(name) => format!(
                "the formula defines or redefines commands with `{name}`, so no partial match has provable structure"
            ),
        }
    }
}

/// Commands the pinned renderer accepts that mutate the macro namespace.
/// A formula containing one has no statically provable structure (any
/// command, symbols included, may be redefined mid-formula), so only the
/// whole formula stays blankable. Sorted for binary search.
const DEFINING_COMMANDS: &[&str] = &[
    r"\def",
    r"\edef",
    r"\futurelet",
    r"\gdef",
    r"\global",
    r"\let",
    r"\long",
    r"\newcommand",
    r"\providecommand",
    r"\renewcommand",
    r"\xdef",
];

/// Zero-argument symbol commands a span may contain (ruled 2026-08-31,
/// ADR 0040). ADDITIVE ONLY: removing an entry rejects decks it accepted.
const BLANKABLE_SYMBOLS: &[&str] = &[
    r"\Delta",
    r"\Gamma",
    r"\Im",
    r"\Lambda",
    r"\Leftarrow",
    r"\Leftrightarrow",
    r"\Omega",
    r"\Phi",
    r"\Pi",
    r"\Psi",
    r"\Re",
    r"\Rightarrow",
    r"\Sigma",
    r"\Theta",
    r"\Upsilon",
    r"\Xi",
    r"\aleph",
    r"\alpha",
    r"\angle",
    r"\approx",
    r"\ast",
    r"\beta",
    r"\bigcap",
    r"\bigcup",
    r"\bullet",
    r"\cap",
    r"\cdot",
    r"\chi",
    r"\circ",
    r"\cong",
    r"\cos",
    r"\cup",
    r"\delta",
    r"\div",
    r"\downarrow",
    r"\ell",
    r"\emptyset",
    r"\epsilon",
    r"\equiv",
    r"\eta",
    r"\exists",
    r"\exp",
    r"\forall",
    r"\gamma",
    r"\geq",
    r"\gets",
    r"\gg",
    r"\hbar",
    r"\in",
    r"\infty",
    r"\int",
    r"\iota",
    r"\kappa",
    r"\lambda",
    r"\leftarrow",
    r"\leftrightarrow",
    r"\leq",
    r"\lim",
    r"\ll",
    r"\ln",
    r"\log",
    r"\mapsto",
    r"\max",
    r"\mid",
    r"\min",
    r"\mp",
    r"\mu",
    r"\nabla",
    r"\neg",
    r"\neq",
    r"\ni",
    r"\notin",
    r"\nu",
    r"\odot",
    r"\oint",
    r"\omega",
    r"\ominus",
    r"\oplus",
    r"\oslash",
    r"\otimes",
    r"\parallel",
    r"\partial",
    r"\perp",
    r"\phi",
    r"\pi",
    r"\pm",
    r"\prime",
    r"\prod",
    r"\propto",
    r"\psi",
    r"\rho",
    r"\rightarrow",
    r"\setminus",
    r"\sigma",
    r"\sim",
    r"\simeq",
    r"\sin",
    r"\star",
    r"\subset",
    r"\subseteq",
    r"\sum",
    r"\supset",
    r"\supseteq",
    r"\tan",
    r"\tau",
    r"\theta",
    r"\times",
    r"\to",
    r"\uparrow",
    r"\upsilon",
    r"\varepsilon",
    r"\varnothing",
    r"\varphi",
    r"\varpi",
    r"\varrho",
    r"\varsigma",
    r"\vartheta",
    r"\vee",
    r"\wedge",
    r"\xi",
    r"\zeta",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    Sup,
    Sub,
    Prime,
}

enum Kind {
    Sequence,
    Open,
    Close,
    Structural(char),
    Script(Script),
    Other,
}

struct Token {
    span: Range<usize>,
    kind: Kind,
}

fn tokens(payload: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut chars = payload.char_indices().peekable();
    while let Some((at, ch)) = chars.next() {
        let kind = match ch {
            '\\' => {
                let mut end = at + 1;
                let mut word = false;
                while let Some((next_at, next)) = chars.peek().copied() {
                    if !next.is_ascii_alphabetic() {
                        break;
                    }
                    chars.next();
                    word = true;
                    end = next_at + 1;
                }
                if &payload[at..end] == r"\verb" {
                    if let Some((next_at, '*')) = chars.peek().copied() {
                        chars.next();
                        end = next_at + 1;
                    }
                    if let Some((delimiter_at, delimiter)) = chars.next() {
                        end = delimiter_at + delimiter.len_utf8();
                        for (body_at, body) in chars.by_ref() {
                            end = body_at + body.len_utf8();
                            if body == delimiter {
                                break;
                            }
                        }
                    }
                }
                if !word && let Some((next_at, next)) = chars.peek().copied() {
                    chars.next();
                    end = next_at + next.len_utf8();
                }
                out.push(Token {
                    span: at..end,
                    kind: Kind::Sequence,
                });
                continue;
            }
            '{' => Kind::Open,
            '}' => Kind::Close,
            '^' | '_' | '&' | '%' | '#' => Kind::Structural(ch),
            '\'' => Kind::Script(Script::Prime),
            // The pinned renderer's own classification, so the set cannot
            // drift from what its atom parser converts into scripts.
            ch => match ratex_parser::unicode_sup_sub::unicode_sub_sup(ch) {
                Some((_, true)) => Kind::Script(Script::Sub),
                Some((_, false)) => Kind::Script(Script::Sup),
                None => Kind::Other,
            },
        };
        out.push(Token {
            span: at..at + ch.len_utf8(),
            kind,
        });
    }
    out
}

/// The extents (command end through argument end, braces included) whose
/// contents Ratex lays out but never draws: a blank in there is invisible.
fn invisible_argument_extents(payload: &str, tokens: &[Token]) -> Vec<(Range<usize>, String)> {
    let mut extents = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if !matches!(token.kind, Kind::Sequence) {
            continue;
        }
        let name = &payload[token.span.clone()];
        if !matches!(name, r"\phantom" | r"\hphantom" | r"\vphantom") {
            continue;
        }
        let mut next = index + 1;
        while next < tokens.len()
            && matches!(tokens[next].kind, Kind::Other)
            && payload[tokens[next].span.clone()]
                .chars()
                .all(char::is_whitespace)
        {
            next += 1;
        }
        let Some(argument) = tokens.get(next) else {
            continue;
        };
        let mut end = argument.span.end;
        if matches!(argument.kind, Kind::Open) {
            let mut depth = 0i32;
            for later in &tokens[next..] {
                match later.kind {
                    Kind::Open => depth += 1,
                    Kind::Close => {
                        depth -= 1;
                        if depth == 0 {
                            end = later.span.end;
                            break;
                        }
                    }
                    _ => {}
                }
                end = later.span.end;
            }
        }
        extents.push((token.span.end..end, name.to_string()));
    }
    extents
}

/// Brace-group arity per the pinned renderer: a registered function
/// consumes `num_args` groups, a built-in macro consumes what its
/// definition references, and an unregistered control sequence is a symbol
/// consuming none. A statically unprovable arity absorbs the whole run:
/// over-absorption rejects a partial span instead of masking the wrong unit.
fn command_brace_arity(command: &str) -> usize {
    use ratex_parser::macro_expander::{MacroDefinition, MacroExpander};
    // The allowlist is the zero-arity authority for its own members: its
    // review vets exactly "zero-argument, learner-visible", so a member
    // stays blankable even where the probe cannot prove it.
    if BLANKABLE_SYMBOLS.binary_search(&command).is_ok() {
        return 0;
    }
    if let Some(spec) = ratex_parser::functions::FUNCTIONS.get(command) {
        return spec.num_args;
    }
    match MacroExpander::new("", ratex_parser::Mode::Math).get_macro(command) {
        Some(MacroDefinition::Text(body)) => {
            let arity = referenced_params(body);
            if expansion_saturates(command, arity) {
                arity
            } else {
                usize::MAX
            }
        }
        Some(MacroDefinition::Tokens { num_args, .. }) => *num_args,
        Some(MacroDefinition::Function(_)) => usize::MAX,
        None => 0,
    }
}

/// Highest `#N` a text-macro body references; `##` is a literal `#`
/// (TeX's own rule).
fn referenced_params(body: &str) -> usize {
    let mut arity = 0usize;
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '#' {
            continue;
        }
        match chars.peek() {
            Some('#') => {
                chars.next();
            }
            Some(digit) if digit.is_ascii_digit() => {
                arity = arity.max(digit.to_digit(10).unwrap_or(0) as usize);
                chars.next();
            }
            _ => {}
        }
    }
    arity
}

/// Whether `command`, handed exactly `arity` brace groups and nothing
/// else, parses without demanding more input: the pinned renderer itself
/// is the oracle. Only its end-of-input-wanting-an-argument diagnostic
/// marks a stream consumer (`\operatorname` dispatches past the probe's
/// end); any other outcome, success or an unrelated complaint about the
/// placeholder arguments (`\tmspace` rejects them as sizes), proves the
/// visible arity is all the macro takes. Only the frozen builtin
/// namespace can reach this probe: a formula that mutates the namespace
/// is rejected wholesale before spans are walked.
fn expansion_saturates(command: &str, arity: usize) -> bool {
    let mut probe = String::from(command);
    for _ in 0..arity {
        probe.push_str("{x}");
    }
    match ratex_parser::parser::parse(&probe) {
        Ok(_) => true,
        Err(error) => error.message != "Unexpected end of input in a macro argument",
    }
}

fn whitespace_token(payload: &str, token: &Token) -> bool {
    matches!(token.kind, Kind::Other)
        && payload[token.span.clone()].chars().all(char::is_whitespace)
}

fn previous_content_token(payload: &str, tokens: &[Token], index: usize) -> Option<usize> {
    let mut previous = index.checked_sub(1)?;
    while whitespace_token(payload, &tokens[previous]) {
        previous = previous.checked_sub(1)?;
    }
    Some(previous)
}

fn next_content_token(payload: &str, tokens: &[Token], index: usize) -> Option<usize> {
    let mut next = index + 1;
    while next < tokens.len() && whitespace_token(payload, &tokens[next]) {
        next += 1;
    }
    (next < tokens.len()).then_some(next)
}

/// The atom a script attaches to, walking past sibling scripts and their
/// operands: in `x_i^2` both scripts belong to `x`, never to `i`.
fn operand_before(payload: &str, tokens: &[Token], index: usize) -> Option<Range<usize>> {
    let mut at = index;
    loop {
        let previous = previous_content_token(payload, tokens, at)?;
        if matches!(tokens[previous].kind, Kind::Script(_)) {
            at = previous;
            continue;
        }
        let (mut operand, mut start_index) = if matches!(tokens[previous].kind, Kind::Close) {
            let mut depth = 0i32;
            let mut start = None;
            for (index, earlier) in tokens[..=previous].iter().enumerate().rev() {
                match earlier.kind {
                    Kind::Close => depth += 1,
                    Kind::Open => {
                        depth -= 1;
                        if depth == 0 {
                            start = Some(index);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            match start {
                Some(index) => (tokens[index].span.start..tokens[previous].span.end, index),
                None => (tokens.first()?.span.start..tokens[previous].span.end, 0),
            }
        } else {
            (tokens[previous].span.clone(), previous)
        };
        // A group run is a command's application only when the command's
        // renderer arity consumes the whole run; a shorter arity leaves the
        // trailing groups (the script's base among them) independent.
        if matches!(tokens[previous].kind, Kind::Close) {
            let mut run_start = start_index;
            let mut walked = 1usize;
            while let Some(before) = previous_content_token(payload, tokens, run_start) {
                match tokens[before].kind {
                    Kind::Close => {
                        let mut depth = 0i32;
                        let mut open = None;
                        for (index, earlier) in tokens[..=before].iter().enumerate().rev() {
                            match earlier.kind {
                                Kind::Close => depth += 1,
                                Kind::Open => {
                                    depth -= 1;
                                    if depth == 0 {
                                        open = Some(index);
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                        match open {
                            Some(index) => {
                                run_start = index;
                                walked += 1;
                            }
                            None => break,
                        }
                    }
                    Kind::Sequence => {
                        if walked <= command_brace_arity(&payload[tokens[before].span.clone()]) {
                            operand.start = tokens[before].span.start;
                            start_index = before;
                        }
                        break;
                    }
                    _ => break,
                }
            }
        }
        match previous_content_token(payload, tokens, start_index) {
            Some(owner) if matches!(tokens[owner].kind, Kind::Structural('^' | '_')) => at = owner,
            _ => return Some(operand),
        }
    }
}

fn operand_after(payload: &str, tokens: &[Token], index: usize) -> Option<Range<usize>> {
    let mut next = index + 1;
    while next < tokens.len() && whitespace_token(payload, &tokens[next]) {
        next += 1;
    }
    let first = tokens.get(next)?;
    if matches!(first.kind, Kind::Open) {
        let mut depth = 0i32;
        for later in &tokens[next..] {
            match later.kind {
                Kind::Open => depth += 1,
                Kind::Close => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(first.span.start..later.span.end);
                    }
                }
                _ => {}
            }
        }
        return Some(first.span.start..tokens.last()?.span.end);
    }
    Some(first.span.clone())
}

/// Commands whose argument the pinned renderer parses in TEXT mode, where
/// script spellings are ordinary characters, with the 1-based index of that
/// argument (the earlier arguments of the color boxes are not prose).
const TEXT_ARGUMENT_COMMANDS: &[(&str, usize)] = &[
    (r"\colorbox", 2),
    (r"\emph", 1),
    (r"\fcolorbox", 3),
    (r"\hbox", 1),
    (r"\text", 1),
    (r"\textbf", 1),
    (r"\textit", 1),
    (r"\textmd", 1),
    (r"\textnormal", 1),
    (r"\textrm", 1),
    (r"\textsc", 1),
    (r"\textsf", 1),
    (r"\textsl", 1),
    (r"\texttt", 1),
    (r"\textup", 1),
];

/// The extents of text-mode arguments: script spellings inside them are
/// ordinary visible characters, never scripts.
fn text_argument_extents(payload: &str, tokens: &[Token]) -> Vec<Range<usize>> {
    let mut extents = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if !matches!(token.kind, Kind::Sequence) {
            continue;
        }
        let name = &payload[token.span.clone()];
        let Some((_, text_argument)) = TEXT_ARGUMENT_COMMANDS
            .iter()
            .find(|(command, _)| *command == name)
        else {
            continue;
        };
        let mut cursor = index;
        for argument in 1..=*text_argument {
            let Some(next) = next_content_token(payload, tokens, cursor) else {
                break;
            };
            let end_index = if matches!(tokens[next].kind, Kind::Open) {
                let mut depth = 0i32;
                let mut end = next;
                for (later_index, later) in tokens.iter().enumerate().skip(next) {
                    match later.kind {
                        Kind::Open => depth += 1,
                        Kind::Close => {
                            depth -= 1;
                            if depth == 0 {
                                end = later_index;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                end
            } else {
                next
            };
            if argument == *text_argument {
                extents.push(tokens[next].span.start..tokens[end_index].span.end);
            }
            cursor = end_index;
        }
    }
    extents
}

fn trim_extent(payload: &str, range: &Range<usize>) -> Range<usize> {
    let slice = &payload[range.clone()];
    let start = range.start + (slice.len() - slice.trim_start().len());
    start..start + slice.trim().len()
}

pub(super) fn structural_unit(payload: &str, range: &Range<usize>) -> Result<(), Violation> {
    if trim_extent(payload, &(0..payload.len())) == trim_extent(payload, range) {
        return Ok(());
    }

    let tokens = tokens(payload);
    // An unescaped `%` comments the rest of its line, so a defining
    // spelling there never reaches the renderer's namespace.
    let mut commented = false;
    for token in &tokens {
        if commented {
            commented = !payload[token.span.clone()].contains('\n');
            continue;
        }
        if matches!(token.kind, Kind::Structural('%')) {
            commented = true;
        } else if matches!(token.kind, Kind::Sequence)
            && DEFINING_COMMANDS
                .binary_search(&&payload[token.span.clone()])
                .is_ok()
        {
            return Err(Violation::MutableNamespace(
                payload[token.span.clone()].to_string(),
            ));
        }
    }
    if tokens
        .iter()
        .any(|token| matches!(&token.kind, Kind::Structural('%')) && token.span.end <= range.start)
    {
        return Err(Violation::Comment);
    }
    for (extent, name) in invisible_argument_extents(payload, &tokens) {
        if range.start < extent.end && extent.start < range.end {
            return Err(Violation::NotLearnerVisible(name));
        }
    }
    let text_extents = text_argument_extents(payload, &tokens);
    let mut depth_before = vec![0i32; payload.len() + 1];
    let mut depth = 0i32;
    for token in &tokens {
        for byte in token.span.clone() {
            depth_before[byte] = depth;
        }
        match token.kind {
            Kind::Open => depth += 1,
            Kind::Close => depth -= 1,
            _ => {}
        }
    }
    depth_before[payload.len()] = depth;

    for token in &tokens {
        if !matches!(token.kind, Kind::Sequence) {
            continue;
        }
        for edge in [range.start, range.end] {
            if token.span.start < edge && edge < token.span.end {
                return Err(Violation::EndpointInsideControlSequence(
                    payload[token.span.clone()].to_string(),
                ));
            }
        }
    }

    for (index, token) in tokens.iter().enumerate() {
        if token.span.start < range.start || token.span.end > range.end {
            continue;
        }
        match &token.kind {
            Kind::Structural(ch @ ('^' | '_')) => {
                let complete = operand_before(payload, &tokens, index)
                    .zip(operand_after(payload, &tokens, index))
                    .is_some_and(|(base, script)| {
                        range.start <= base.start && script.end <= range.end
                    });
                if !complete {
                    return Err(Violation::IncompleteScript(*ch));
                }
            }
            Kind::Structural(ch) => return Err(Violation::StructuralToken(*ch)),
            Kind::Script(script) => {
                if text_extents
                    .iter()
                    .any(|extent| extent.start <= token.span.start && token.span.end <= extent.end)
                {
                    continue;
                }
                let mut first = index;
                while let Some(previous) = previous_content_token(payload, &tokens, first) {
                    if matches!(tokens[previous].kind, Kind::Script(other) if other == *script) {
                        first = previous;
                    } else {
                        break;
                    }
                }
                let mut last = index;
                while let Some(next) = next_content_token(payload, &tokens, last) {
                    if matches!(tokens[next].kind, Kind::Script(other) if other == *script) {
                        last = next;
                    } else {
                        break;
                    }
                }
                let mut complete = operand_before(payload, &tokens, first)
                    .is_some_and(|base| range.start <= base.start)
                    && tokens[last].span.end <= range.end;
                // The parser folds a `^`-script after a prime run into the
                // same superscript, so cutting it off cuts the cluster.
                if complete
                    && *script == Script::Prime
                    && let Some(next) = next_content_token(payload, &tokens, last)
                    && matches!(tokens[next].kind, Kind::Structural('^'))
                    && tokens[next].span.start >= range.end
                {
                    complete = false;
                }
                if !complete {
                    let spelled = payload[token.span.clone()].chars().next().unwrap_or('\'');
                    return Err(Violation::IncompleteScript(spelled));
                }
            }
            Kind::Sequence => {
                let text = &payload[token.span.clone()];
                if text == r"\\" {
                    return Err(Violation::RowBreak);
                }
                if text == r"\{" || text == r"\}" || BLANKABLE_SYMBOLS.binary_search(&text).is_ok()
                {
                    continue;
                }
                let argument_continues = tokens
                    .iter()
                    .filter(|later| later.span.start >= range.end)
                    .find(|later| !whitespace_token(payload, later))
                    .is_some_and(|later| matches!(later.kind, Kind::Open));
                if argument_continues {
                    return Err(Violation::IncompleteCommandApplication(text.to_string()));
                }
                return Err(Violation::ControlSequence(text.to_string()));
            }
            _ => {}
        }
    }

    let entry = depth_before[range.start];
    if depth_before[range.end] != entry
        || (range.start + 1..range.end).any(|byte| depth_before[byte] < entry)
    {
        return Err(Violation::GroupSplit);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(payload: &str, hidden: &str) -> Result<(), Violation> {
        let start = payload.find(hidden).expect("fixture hidden text present");
        structural_unit(payload, &(start..start + hidden.len()))
    }

    #[test]
    fn a_whole_payload_match_bypasses_every_token_rule() {
        assert_eq!(Ok(()), unit(r"x^2", r"x^2"));
        assert_eq!(
            Ok(()),
            unit(
                r"\begin{pmatrix}a & b\end{pmatrix}",
                r"\begin{pmatrix}a & b\end{pmatrix}"
            ),
        );
    }

    #[test]
    fn outer_whitespace_trims_on_both_sides_of_the_comparison() {
        assert_eq!(Ok(()), unit(r" \gamma ", r"\gamma"));
        assert_eq!(
            Ok(()),
            structural_unit(" \\gamma ", &(0..8)),
            "a match carrying the outer whitespace behaves like the trimmed form"
        );
    }

    #[test]
    fn every_match_endpoint_inside_a_control_word_is_named() {
        for hidden in ["q", "eq", r"\le"] {
            assert_eq!(
                Err(Violation::EndpointInsideControlSequence(
                    r"\leq".to_string()
                )),
                unit(r"x \leq y", hidden),
                "{hidden}"
            );
        }
    }

    #[test]
    fn verbatim_math_is_one_indivisible_sequence() {
        for payload in [r"\verb|target| x", r"\verb*étargeté x"] {
            assert!(
                matches!(
                    unit(payload, "target"),
                    Err(Violation::EndpointInsideControlSequence(_))
                ),
                "{payload}"
            );
        }
        assert_eq!(Ok(()), unit(r"\verb|a| target", "target"));
    }

    #[test]
    fn every_unescaped_structural_token_is_rejected_inside_a_match() {
        for (payload, hidden, token) in [
            (r"a & b", r"a &", '&'),
            (r"a % b", r"a %", '%'),
            (r"a # b", r"a #", '#'),
        ] {
            assert_eq!(
                Err(Violation::StructuralToken(token)),
                unit(payload, hidden),
                "{payload} / {hidden}"
            );
        }
    }

    #[test]
    fn source_after_an_unescaped_percent_is_a_comment_not_a_unit() {
        assert_eq!(Err(Violation::Comment), unit("a % target", "target"));
        assert_eq!(Ok(()), unit(r"a \% target", "target"));
    }

    #[test]
    fn every_phantom_argument_is_invisible_to_matches() {
        for command in [r"\phantom", r"\hphantom", r"\vphantom"] {
            assert_eq!(
                Err(Violation::NotLearnerVisible(command.to_string())),
                unit(&format!("{command}{{target}} x"), "target"),
                "{command}"
            );
        }
        assert_eq!(
            Err(Violation::NotLearnerVisible(r"\phantom".to_string())),
            unit(r"\phantom{target} x", r"{target}"),
            "the braces belong to the invisible argument too"
        );
        assert_eq!(
            Err(Violation::NotLearnerVisible(r"\phantom".to_string())),
            unit(r"\phantom x y", "x"),
            "a braceless single-token argument is equally invisible"
        );
        assert_eq!(
            Ok(()),
            unit(r"\phantom{a} target", "target"),
            "visible text after the phantom stays bindable"
        );
    }

    #[test]
    fn a_row_break_inside_a_match_is_rejected() {
        assert_eq!(Err(Violation::RowBreak), unit(r"a \\ b", r"a \\"));
    }

    #[test]
    fn a_contained_command_is_rejected_naming_it() {
        assert_eq!(
            Err(Violation::ControlSequence(r"\quad".to_string())),
            unit(r"x + \quad + y", r"+ \quad +")
        );
        assert_eq!(
            Err(Violation::ControlSequence(r"\left".to_string())),
            unit(r"\left( x \right)", r"\left( x")
        );
    }

    #[test]
    fn every_blankable_symbol_consumes_no_brace_groups() {
        use ratex_parser::macro_expander::{MacroDefinition, MacroExpander};
        for symbol in BLANKABLE_SYMBOLS {
            assert_eq!(0, command_brace_arity(symbol), "{symbol} is pinned zero");
            // The mechanical half of the review: where the renderer states
            // an arity, it must agree; a text macro's body walk must find
            // zero (proven self-saturated) or unprovable (the vetting then
            // carries it), never a positive arity contradicting the review.
            if let Some(spec) = ratex_parser::functions::FUNCTIONS.get(symbol) {
                assert_eq!(0, spec.num_args, "{symbol} consumes renderer arguments");
            } else {
                let expander = MacroExpander::new("", ratex_parser::Mode::Math);
                match expander.get_macro(symbol) {
                    Some(MacroDefinition::Tokens { num_args, .. }) => {
                        assert_eq!(0, *num_args, "{symbol} consumes macro arguments");
                    }
                    Some(MacroDefinition::Function(_)) => {
                        panic!("{symbol} is Function-backed; its arity is unprovable");
                    }
                    Some(MacroDefinition::Text(body)) => {
                        let arity = referenced_params(body);
                        assert!(
                            arity == 0 || !expansion_saturates(symbol, arity),
                            "{symbol} probes to arity {arity}, contradicting the review"
                        );
                    }
                    None => {}
                }
            }
        }
    }

    #[test]
    fn the_probed_consumers_are_unprovable_and_every_disputed_family_is_exact() {
        // The four stream-consumers per Codex's exhaustive EOF probe of the
        // pinned macro table (2026-08-31); production runs the same probe
        // per command, so these rows pin the oracle, not a transcription.
        for name in [r"\Braket", r"\set", r"\Set", r"\operatorname"] {
            assert_eq!(
                usize::MAX,
                command_brace_arity(name),
                "{name} consumes the stream"
            );
        }
        // Every family an earlier arity rule ever misclassified, plus the
        // saturated samples: exact arities per the same probe.
        for (name, arity) in [
            (r"\boxed", 1),
            (r"\hphantom", 1),
            (r"\varGamma", 0),
            (r"\pod", 1),
            (r"\pmod", 1),
            (r"\mod", 1),
            (r"\tmspace", 3),
            (r"\limsup", 0),
            (r"\liminf", 0),
            (r"\injlim", 0),
            (r"\projlim", 0),
            (r"\varlimsup", 0),
            (r"\varliminf", 0),
            (r"\varinjlim", 0),
            (r"\varprojlim", 0),
            (r"\argmin", 0),
            (r"\argmax", 0),
            (r"\plim", 0),
            (r"\iff", 0),
            (r"\implies", 0),
            (r"\impliedby", 0),
            (r"\not", 0),
            (r"\minuso", 0),
            (r"\coloneqq", 0),
            (r"\dblcolon", 0),
            (r"\vcentcolon", 0),
            (r"\,", 0),
            (r"\;", 0),
            (r"\!", 0),
            (r"\:", 0),
            (r"\thinspace", 0),
            (r"\cr", 0),
            (r"\newline", 0),
        ] {
            assert_eq!(arity, command_brace_arity(name), "{name} saturates in-body");
        }
    }

    #[test]
    fn the_blankable_symbol_table_is_sorted_unique_and_lexer_shaped() {
        assert!(
            BLANKABLE_SYMBOLS.windows(2).all(|pair| pair[0] < pair[1]),
            "binary_search requires strict ascending order"
        );
        for name in BLANKABLE_SYMBOLS {
            assert!(
                name.starts_with('\\') && name[1..].chars().all(|ch| ch.is_ascii_alphabetic()),
                "{name} is not the lexer's control-word shape"
            );
        }
    }

    #[test]
    fn an_allowlisted_symbol_command_is_a_blankable_unit() {
        assert_eq!(Ok(()), unit(r"x = -b \pm \sqrt{d}", r"\pm"));
        assert_eq!(Ok(()), unit(r"e^{i\pi} + 1 = 0", r"\pi"));
        assert_eq!(Ok(()), unit(r"a \leq b", r"\leq"));
    }

    #[test]
    fn a_complete_base_and_script_is_a_blankable_unit() {
        assert_eq!(Ok(()), unit(r"x = b^2 - 4ac", r"b^2"));
        assert_eq!(Ok(()), unit(r"a_i + b", r"a_i"));
        assert_eq!(Ok(()), unit(r"{ab}^{n+1} + c", r"{ab}^{n+1}"));
    }

    #[test]
    fn a_script_without_its_whole_base_and_script_is_incomplete() {
        assert_eq!(
            Err(Violation::IncompleteScript('^')),
            unit(r"x^2 + y", r"^2")
        );
        assert_eq!(
            Err(Violation::IncompleteScript('^')),
            unit(r"x^2 + y", r"x^")
        );
        assert_eq!(
            Err(Violation::IncompleteScript('_')),
            unit(r"a_i + b", r"a_")
        );
    }

    #[test]
    fn a_cut_command_application_is_named_when_its_argument_continues() {
        assert_eq!(
            Err(Violation::IncompleteCommandApplication(
                r"\frac".to_string()
            )),
            unit(r"z+\frac{a}{b}", r"\frac{a}"),
            "the orphaned-denominator counterexample (ADR 0040 adversary)"
        );
    }

    #[test]
    fn a_unicode_script_needs_its_base_and_its_whole_cluster() {
        assert_eq!(
            Err(Violation::IncompleteScript('\u{00B2}')),
            unit("x\u{00B2} + y", "\u{00B2}"),
            "a superscript two alone orphans its base"
        );
        assert_eq!(
            Err(Violation::IncompleteScript('\u{00B2}')),
            unit("x\u{00B2}\u{00B3} + y", "x\u{00B2}"),
            "cutting the cluster after the two orphans the three"
        );
        assert_eq!(
            Err(Violation::IncompleteScript('\u{2081}')),
            unit("a\u{2081} + b", "\u{2081}"),
            "a subscript one alone orphans its base"
        );
        assert_eq!(Ok(()), unit("x\u{00B2} + y", "x\u{00B2}"));
        assert_eq!(Ok(()), unit("x\u{00B2}\u{00B3} + y", "x\u{00B2}\u{00B3}"));
    }

    #[test]
    fn a_prime_run_needs_its_base_and_its_whole_cluster() {
        assert_eq!(
            Err(Violation::IncompleteScript('\'')),
            unit("x' + y", "'"),
            "a prime alone orphans its base"
        );
        assert_eq!(
            Err(Violation::IncompleteScript('\'')),
            unit("x'' + y", "x'"),
            "cutting a prime run leaves an orphaned prime"
        );
        assert_eq!(
            Err(Violation::IncompleteScript('\'')),
            unit("x'^2 + y", "x'"),
            "the ^-script joins the prime cluster, so it cannot be cut off"
        );
        assert_eq!(Ok(()), unit("x' + y", "x'"));
        assert_eq!(Ok(()), unit("x'' + y", "x''"));
        assert_eq!(Ok(()), unit("x'^2 + y", "x'^2"));
    }

    #[test]
    fn a_text_mode_argument_keeps_script_spellings_ordinary() {
        assert_eq!(Ok(()), unit("\\text{x'} + y", "'"));
        assert_eq!(Ok(()), unit("\\text{x\u{00B2}} + y", "\u{00B2}"));
        assert_eq!(Ok(()), unit("\\textbf{x'} + y", "x'"));
        assert_eq!(
            Ok(()),
            unit("\\colorbox{red}{x'} + y", "'"),
            "the text argument is the second one; the color argument is not prose"
        );
    }

    #[test]
    fn a_script_after_a_command_application_walks_to_the_command() {
        assert_eq!(
            Err(Violation::IncompleteScript('^')),
            unit(r"z+\frac{a}{b}^2", r"{b}^2"),
            "the exponent belongs to the whole fraction atom"
        );
        assert_eq!(
            Ok(()),
            unit(r"x + {a}{b}^2", r"{b}^2"),
            "bare juxtaposed groups are not a command application"
        );
    }

    #[test]
    fn a_group_after_an_argument_taking_macro_walks_to_the_macro() {
        assert_eq!(
            Err(Violation::IncompleteScript('^')),
            unit(r"z+\pmod{x}^2", r"{x}^2")
        );
        assert_eq!(
            Err(Violation::ControlSequence(r"\pmod".into())),
            unit(r"z+\pmod{x}^2", r"\pmod{x}^2"),
            "the allowlist still owns the atom; whole-formula stays the remedy"
        );
    }

    #[test]
    fn a_group_after_a_locally_defined_macro_walks_to_the_macro() {
        assert_eq!(
            Err(Violation::MutableNamespace(r"\def".into())),
            unit(r"\def\foo#1{(#1)}\foo{x}^2", r"{x}^2")
        );
    }

    #[test]
    fn a_defining_formula_still_blanks_whole() {
        assert_eq!(
            Ok(()),
            unit(r"\def\foo#1{(#1)}\foo{x}^2", r"\def\foo#1{(#1)}\foo{x}^2")
        );
    }

    #[test]
    fn a_group_after_a_dispatch_macro_walks_to_the_macro() {
        assert_eq!(
            Err(Violation::IncompleteScript('^')),
            unit(r"z+\operatorname{x}^2", r"{x}^2")
        );
    }

    #[test]
    fn a_group_after_a_fixed_arity_text_macro_stays_independent() {
        assert_eq!(Ok(()), unit(r"z+\boxed{x}{y}^2", r"{y}^2"));
    }

    #[test]
    fn a_group_after_a_saturated_operator_macro_stays_independent() {
        assert_eq!(Ok(()), unit(r"z+\limsup{x}^2", r"{x}^2"));
    }

    #[test]
    fn a_defining_spelling_in_a_later_comment_does_not_mutate_the_namespace() {
        assert_eq!(Ok(()), unit(r"x+y % \def\foo{z}", "x"));
    }

    #[test]
    fn a_comment_ends_at_its_line_so_a_defining_spelling_below_still_mutates() {
        assert_eq!(
            Err(Violation::MutableNamespace(r"\def".into())),
            unit("x+y % c\n\\def\\bar{z} w", "x")
        );
    }

    #[test]
    fn the_defining_command_table_is_sorted_and_unique() {
        for pair in DEFINING_COMMANDS.windows(2) {
            assert!(pair[0] < pair[1], "{pair:?} out of order");
        }
    }

    #[test]
    fn a_group_after_a_zero_argument_symbol_remains_its_own_script_base() {
        assert_eq!(Ok(()), unit(r"z+\alpha{x}^2", r"{x}^2"));
    }

    #[test]
    fn a_stacked_script_walks_to_the_owning_atom() {
        assert_eq!(
            Err(Violation::IncompleteScript('^')),
            unit("x_i^2 + y", "i^2"),
            "both scripts of x_i^2 attach to x, so i is not a base"
        );
        assert_eq!(Ok(()), unit("x_i^2 + y", "x_i^2"));
    }

    #[test]
    fn escaped_braces_are_literal_inside_and_as_matches() {
        assert_eq!(Ok(()), unit(r"A=\{x\}", r"\{x\}"));
        assert_eq!(Ok(()), unit(r"A=\{x\}", r"x"));
    }

    #[test]
    fn a_match_splitting_real_brace_groups_is_rejected() {
        assert_eq!(
            Err(Violation::GroupSplit),
            unit(r"\frac{ab}{cd}", r"ab}{cd")
        );
        assert_eq!(Err(Violation::GroupSplit), unit(r"{ab}", r"{ab"));
        assert_eq!(Err(Violation::GroupSplit), unit(r"a{b}c", r"b}c"));
    }

    #[test]
    fn an_equal_depth_group_interior_is_a_unit() {
        assert_eq!(Ok(()), unit(r"\frac{ab}{cd}", r"ab"));
        assert_eq!(Ok(()), unit(r"x^{n+1}", r"n+1"));
    }
}
