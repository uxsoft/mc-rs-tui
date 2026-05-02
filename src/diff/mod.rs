//! Line-level diff between two text buffers, oriented for side-by-side rendering.
//!
//! We intentionally produce a "rendered" data structure rather than streaming
//! sink output: a [`DiffModel`] holds the ordered list of [`Row`]s where each
//! row is one of `Same`/`Removed`/`Added`/`Changed(L, R)`. The diff viewer
//! widget walks the rows directly to draw side-by-side hunks.

use imara_diff::{Algorithm, Diff, InternedInput, Token};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// Identical line on both sides.
    Same(String),
    /// Line only on the left (removed in right).
    Removed(String),
    /// Line only on the right (added).
    Added(String),
    /// A line that appears on both sides but with different content
    /// (synthesised from a `Removed`+`Added` pair within one hunk).
    Changed(String, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// Index in `rows` of the first row in this hunk.
    pub start: usize,
    /// One past the last row.
    pub end: usize,
}

#[derive(Debug, Clone)]
pub struct DiffModel {
    pub rows: Vec<Row>,
    pub hunks: Vec<Hunk>,
}

impl DiffModel {
    #[must_use]
    pub fn build(left: &str, right: &str) -> Self {
        let input = InternedInput::new(left, right);
        let mut diff = Diff::compute(Algorithm::Histogram, &input);
        diff.postprocess_lines(&input);

        let mut rows: Vec<Row> = Vec::new();
        let mut hunks: Vec<Hunk> = Vec::new();
        let mut left_idx: u32 = 0;
        let mut right_idx: u32 = 0;
        let total_left = input.before.len() as u32;
        let total_right = input.after.len() as u32;

        let token_str = |tok: Token| -> String { input.interner[tok].to_string() };

        for h in diff.hunks() {
            // Emit context (matching) lines from the last position to the start
            // of this hunk.
            while left_idx < h.before.start && right_idx < h.after.start {
                let ltok = input.before[left_idx as usize];
                rows.push(Row::Same(token_str(ltok)));
                left_idx += 1;
                right_idx += 1;
            }

            let hunk_start = rows.len();
            // Pair removals and additions where possible to produce `Changed`.
            let mut i = h.before.start;
            let mut j = h.after.start;
            while i < h.before.end && j < h.after.end {
                let l = token_str(input.before[i as usize]);
                let r = token_str(input.after[j as usize]);
                rows.push(Row::Changed(l, r));
                i += 1;
                j += 1;
            }
            while i < h.before.end {
                rows.push(Row::Removed(token_str(input.before[i as usize])));
                i += 1;
            }
            while j < h.after.end {
                rows.push(Row::Added(token_str(input.after[j as usize])));
                j += 1;
            }
            let hunk_end = rows.len();
            if hunk_end > hunk_start {
                hunks.push(Hunk {
                    start: hunk_start,
                    end: hunk_end,
                });
            }
            left_idx = h.before.end;
            right_idx = h.after.end;
        }

        // Remaining trailing context.
        while left_idx < total_left && right_idx < total_right {
            let ltok = input.before[left_idx as usize];
            rows.push(Row::Same(token_str(ltok)));
            left_idx += 1;
            right_idx += 1;
        }

        Self { rows, hunks }
    }

    /// Whether the two inputs are identical.
    #[must_use]
    pub fn is_equal(&self) -> bool {
        self.hunks.is_empty()
    }

    /// Find the index of the next hunk after `row`, wrapping at the end.
    #[must_use]
    pub fn next_hunk(&self, row: usize) -> Option<usize> {
        self.hunks
            .iter()
            .find(|h| h.start > row)
            .map(|h| h.start)
            .or_else(|| self.hunks.first().map(|h| h.start))
    }

    /// Find the index of the previous hunk before `row`, wrapping at the start.
    #[must_use]
    pub fn prev_hunk(&self, row: usize) -> Option<usize> {
        self.hunks
            .iter()
            .rev()
            .find(|h| h.end <= row)
            .map(|h| h.start)
            .or_else(|| self.hunks.last().map(|h| h.start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_inputs_have_no_hunks() {
        let m = DiffModel::build("a\nb\nc\n", "a\nb\nc\n");
        assert!(m.is_equal());
        assert!(m.rows.iter().all(|r| matches!(r, Row::Same(_))));
    }

    #[test]
    fn pure_addition() {
        let m = DiffModel::build("a\nb\n", "a\nb\nc\n");
        assert!(!m.is_equal());
        assert!(m.rows.iter().any(|r| matches!(r, Row::Added(_))));
    }

    #[test]
    fn pure_removal() {
        let m = DiffModel::build("a\nb\nc\n", "a\nb\n");
        assert!(m.rows.iter().any(|r| matches!(r, Row::Removed(_))));
    }

    #[test]
    fn change_pairs_to_changed_row() {
        let m = DiffModel::build("hello\n", "world\n");
        assert_eq!(m.rows.len(), 1);
        match &m.rows[0] {
            Row::Changed(l, r) => {
                assert!(l.starts_with("hello"));
                assert!(r.starts_with("world"));
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(m.hunks.len(), 1);
    }

    #[test]
    fn hunk_navigation() {
        let m = DiffModel::build("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!(m.hunks.len(), 1);
        assert_eq!(m.next_hunk(0), Some(m.hunks[0].start));
        assert_eq!(m.prev_hunk(m.rows.len()), Some(m.hunks[0].start));
    }
}
