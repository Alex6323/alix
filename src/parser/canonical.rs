use std::hash::Hasher;

use twox_hash::XxHash64;

use super::{closes_fence, collapse, fence_opener};

// A change here stales every persisted content fingerprint: deliberate, never
// a silent refactor.
pub fn canonical_content(front: &str, back: &[String]) -> String {
    let mut out = collapse(&crate::inline::strip_inline(front));
    let mut fence: Option<char> = None;
    let mut prose = String::new();
    for line in back {
        if let Some(ch) = fence {
            out.push('\n');
            out.push_str(line);
            if closes_fence(line, ch) {
                fence = None;
            }
        } else if let Some(ch) = fence_opener(line) {
            if !prose.is_empty() {
                out.push('\n');
                out.push_str(&prose);
                prose.clear();
            }
            out.push('\n');
            out.push_str(line);
            fence = Some(ch);
        } else {
            let collapsed = collapse(&crate::inline::strip_inline(line));
            if !collapsed.is_empty() {
                if !prose.is_empty() {
                    prose.push(' ');
                }
                prose.push_str(&collapsed);
            }
        }
    }
    if !prose.is_empty() {
        out.push('\n');
        out.push_str(&prose);
    }
    out
}

pub fn content_fingerprint(front: &str, back: &[String]) -> u64 {
    let mut hasher = XxHash64::default();
    hasher.write(canonical_content(front, back).as_bytes());
    hasher.finish()
}

pub(super) fn hash64(s: &str) -> u64 {
    let mut hasher = XxHash64::default();
    hasher.write(s.as_bytes());
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_is_stripped_before_fingerprinting() {
        let with = content_fingerprint("q", &["**Paris**".to_string()]);
        let without = content_fingerprint("q", &["Paris".to_string()]);
        assert_eq!(
            with, without,
            "inline markup must not change the content fingerprint"
        );
    }

    #[test]
    fn math_delimiters_do_not_change_the_content_fingerprint() {
        let inline = content_fingerprint("q", &["$x^2$".to_string()]);
        let display = content_fingerprint("q", &["$$x^2$$".to_string()]);
        let plain = content_fingerprint("q", &["x^2".to_string()]);
        assert_eq!(inline, plain);
        assert_eq!(display, plain);
    }
}
