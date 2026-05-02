//! Tiny glob matcher: `*` matches any chars, `?` matches one char,
//! case-insensitive. Used by panel filters and select/unselect group dialogs.

/// Returns `true` if `pattern` matches `text` (case-insensitive).
#[must_use]
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let t: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    glob_inner(&p, &t)
}

fn glob_inner(p: &[char], t: &[char]) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star_p, mut star_t): (Option<usize>, usize) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_p = Some(pi);
            star_t = ti;
            pi += 1;
        } else if let Some(sp) = star_p {
            pi = sp + 1;
            star_t += 1;
            ti = star_t;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*.txt", "foo.txt"));
        assert!(glob_match("*.txt", "FOO.TXT"));
        assert!(glob_match("?oo.txt", "foo.txt"));
        assert!(glob_match("foo*bar", "fooXYZbar"));
        assert!(!glob_match("*.txt", "foo.md"));
        assert!(!glob_match("foo", "bar"));
        assert!(glob_match("", ""));
        assert!(!glob_match("", "x"));
    }
}
