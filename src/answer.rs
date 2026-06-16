//! Answer checking for the answer modes.
//!
//! - **Typing**: the answer must be typed character by character with live
//!   feedback ([`TypingValidator`]). Revealing hints marks the card failed.
//! - **Fuzzy**: a whole line is typed and submitted, then compared with a
//!   typo tolerance ([`grade_fuzzy`]).
//! - **Flip**: the user reveals the answer and grades themselves; no checking
//!   happens here.
//! - **Choice**: the user picks the answer out of four options (see the
//!   [`choice`](crate::choice) module).

/// Which answer mode a review session uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum Mode {
    /// Reveal the answer and grade yourself (again / good / easy).
    #[default]
    Flip,
    /// Type the answer exactly, character by character (the original mode).
    Typing,
    /// Type the answer and submit; small typos are tolerated.
    Fuzzy,
    /// Pick the right answer out of four; the wrong options are sampled from
    /// the other cards of the session.
    Choice,
    /// Reveal the back one line at a time (useful for lyrics or poems), then
    /// grade yourself once the whole card is uncovered.
    #[value(name = "line")]
    LineByLine,
}

/// The state of one typed character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Typed {
    /// The character the user typed.
    pub ch: char,
    /// Whether it matches the expected character at its position.
    pub correct: bool,
}

/// Validates one answer line typed character by character.
///
/// The user can type past mistakes, but the line only completes once every
/// position holds the correct character, so wrong characters must be removed
/// with backspace. Asking for a hint reveals upcoming characters; any hint
/// marks the line as failed (the original allowed 0 hints per line, too).
#[derive(Clone, Debug)]
pub struct TypingValidator {
    expected: Vec<char>,
    typed: Vec<Typed>,
    hints_used: usize,
    typos: usize,
}

/// How many characters a single hint request reveals.
pub const HINT_CHARS: usize = 2;

impl TypingValidator {
    /// Creates a validator for the expected line.
    pub fn new(expected: &str) -> Self {
        Self {
            expected: expected.chars().collect(),
            typed: Vec::new(),
            hints_used: 0,
            typos: 0,
        }
    }

    /// The expected line.
    pub fn expected(&self) -> String {
        self.expected.iter().collect()
    }

    /// The characters typed so far with their correctness.
    pub fn typed(&self) -> &[Typed] {
        &self.typed
    }

    /// Types one character. Returns whether it was correct. Input beyond the
    /// expected length is ignored (and reported as incorrect).
    pub fn type_char(&mut self, ch: char) -> bool {
        if self.typed.len() >= self.expected.len() {
            return false;
        }
        let correct = self.expected[self.typed.len()] == ch;
        if !correct {
            self.typos += 1;
        }
        self.typed.push(Typed { ch, correct });
        correct
    }

    /// Removes the last typed character. Returns `true` if one was removed.
    pub fn backspace(&mut self) -> bool {
        self.typed.pop().is_some()
    }

    /// Reveals the next [`HINT_CHARS`] expected characters. Any incorrectly
    /// typed tail is removed first, so the hint always shows what should
    /// follow the correct prefix. Using a hint marks the line failed.
    pub fn hint(&mut self) -> String {
        self.hints_used += 1;
        if let Some(first_bad) = self.typed.iter().position(|t| !t.correct) {
            self.typed.truncate(first_bad);
        }
        self.expected.iter().skip(self.typed.len()).take(HINT_CHARS).collect()
    }

    /// The not-yet-typed remainder of the expected line.
    pub fn remaining(&self) -> String {
        self.expected.iter().skip(self.typed.len()).collect()
    }

    /// `true` once the whole line has been typed correctly.
    pub fn is_complete(&self) -> bool {
        self.typed.len() == self.expected.len() && self.typed.iter().all(|t| t.correct)
    }

    /// `true` if the line was completed without hints.
    pub fn passed(&self) -> bool {
        self.is_complete() && self.hints_used == 0
    }

    /// Number of hints requested.
    pub fn hints_used(&self) -> usize {
        self.hints_used
    }

    /// Number of incorrect characters typed (including later corrected ones).
    pub fn typos(&self) -> usize {
        self.typos
    }
}

/// The result of grading one line in fuzzy mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuzzyResult {
    /// What the user typed.
    pub input: String,
    /// The expected line.
    pub expected: String,
    /// Levenshtein distance between input and expected.
    pub distance: usize,
    /// Whether the line counts as correct under the given tolerance.
    pub passed: bool,
}

/// Grades a fuzzily typed line. `max_typos` is the maximum tolerated
/// Levenshtein distance per line.
pub fn grade_fuzzy(input: &str, expected: &str, max_typos: usize) -> FuzzyResult {
    let input = input.trim();
    let distance = strsim::levenshtein(input, expected);
    FuzzyResult {
        input: input.to_string(),
        expected: expected.to_string(),
        distance,
        passed: distance <= max_typos,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_correct_line_passes() {
        let mut v = TypingValidator::new("hello");
        for c in "hello".chars() {
            assert!(v.type_char(c));
        }
        assert!(v.is_complete());
        assert!(v.passed());
        assert_eq!(0, v.typos());
    }

    #[test]
    fn typing_incomplete_line_is_not_complete() {
        let mut v = TypingValidator::new("hello");
        v.type_char('h');
        assert!(!v.is_complete());
        assert_eq!("ello", v.remaining());
    }

    #[test]
    fn typing_wrong_char_blocks_completion_until_fixed() {
        let mut v = TypingValidator::new("hello");
        assert!(v.type_char('h'));
        assert!(!v.type_char('3'));
        v.type_char('l');
        v.type_char('l');
        v.type_char('o');
        assert!(!v.is_complete()); // '3' is still wrong at position 1

        // Fix it: backspace everything down to the mistake, retype.
        for _ in 0..4 {
            v.backspace();
        }
        for c in "ello".chars() {
            assert!(v.type_char(c));
        }
        assert!(v.is_complete());
        assert!(v.passed()); // typos don't fail the card, hints do
        assert_eq!(1, v.typos());
    }

    #[test]
    fn typing_input_beyond_length_ignored() {
        let mut v = TypingValidator::new("hi");
        v.type_char('h');
        v.type_char('i');
        assert!(!v.type_char('!'));
        assert!(v.is_complete());
        assert_eq!(2, v.typed().len());
    }

    #[test]
    fn backspace_on_empty_returns_false() {
        let mut v = TypingValidator::new("hi");
        assert!(!v.backspace());
    }

    #[test]
    fn hint_reveals_next_chars_and_fails_the_line() {
        let mut v = TypingValidator::new("hello");
        v.type_char('h');
        assert_eq!("el", v.hint());
        for c in "ello".chars() {
            v.type_char(c);
        }
        assert!(v.is_complete());
        assert!(!v.passed());
        assert_eq!(1, v.hints_used());
    }

    #[test]
    fn hint_clears_incorrect_tail_first() {
        let mut v = TypingValidator::new("hello");
        v.type_char('h');
        v.type_char('3'); // wrong
        v.type_char('l');
        assert_eq!("el", v.hint());
        // The wrong tail ('3' and everything after) is gone.
        assert_eq!(1, v.typed().len());
        assert_eq!("ello", v.remaining());
    }

    #[test]
    fn hint_at_end_is_empty() {
        let mut v = TypingValidator::new("hi");
        v.type_char('h');
        v.type_char('i');
        assert_eq!("", v.hint());
    }

    #[test]
    fn unicode_is_handled_per_char() {
        let mut v = TypingValidator::new("héllö");
        for c in "héllö".chars() {
            assert!(v.type_char(c));
        }
        assert!(v.passed());
    }

    #[test]
    fn fuzzy_exact_match_passes_with_zero_tolerance() {
        let r = grade_fuzzy("hello", "hello", 0);
        assert!(r.passed);
        assert_eq!(0, r.distance);
    }

    #[test]
    fn fuzzy_within_tolerance_passes() {
        let r = grade_fuzzy("helo", "hello", 2);
        assert!(r.passed);
        assert_eq!(1, r.distance);

        let r = grade_fuzzy("hxllo wxrld", "hello world", 2);
        assert!(r.passed);
        assert_eq!(2, r.distance);
    }

    #[test]
    fn fuzzy_beyond_tolerance_fails() {
        let r = grade_fuzzy("hxlxo wxrld", "hello world", 2);
        assert!(!r.passed);
        assert_eq!(3, r.distance);
    }

    #[test]
    fn fuzzy_input_is_trimmed() {
        let r = grade_fuzzy("  hello  ", "hello", 0);
        assert!(r.passed);
    }
}
