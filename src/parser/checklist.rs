//! GFM task-list line recognition, shared by the checkbox-card parser and the
//! checklist renderer. `[x]`/`[X]` = checked, `[ ]` = unchecked (frozen grammar).

/// `Some((checked, option_text))` when `line` is a GFM task-list item, else `None`.
/// The text is the remainder after the marker and its trailing space, not trimmed
/// further (option text keeps its inner spacing; callers strip inline markup).
pub fn parse_line(line: &str) -> Option<(bool, &str)> {
    let mut chars = line.char_indices();
    let (_, bullet) = chars.next()?;
    if !matches!(bullet, '-' | '*' | '+') {
        return None;
    }
    let rest = line.get(bullet.len_utf8()..)?;
    let rest = rest.strip_prefix(' ')?;
    let (checked, after) = if let Some(after) = rest.strip_prefix("[ ] ") {
        (false, after)
    } else if let Some(after) = rest
        .strip_prefix("[x] ")
        .or_else(|| rest.strip_prefix("[X] "))
    {
        (true, after)
    } else {
        return None;
    };
    if after.is_empty() {
        return None;
    }
    Some((checked, after))
}

#[cfg(test)]
mod tests {
    use super::parse_line;

    #[test]
    fn recognizes_all_gfm_bullets_and_both_cases() {
        assert_eq!(Some((false, "four")), parse_line("- [ ] four"));
        assert_eq!(Some((true, "five")), parse_line("* [x] five"));
        assert_eq!(Some((true, "seven")), parse_line("+ [X] seven"));
    }

    #[test]
    fn rejects_non_task_list_lines() {
        assert_eq!(None, parse_line("- a plain bullet"));
        assert_eq!(None, parse_line("just prose"));
        assert_eq!(None, parse_line("-[x] no space after bullet"));
        assert_eq!(None, parse_line("- [x] ".trim_end()));
        assert_eq!(None, parse_line("  - [ ] indented"));
    }
}
