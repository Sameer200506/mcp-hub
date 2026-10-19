//! Minimal line diff (LCS based) used by the configuration preview.
//!
//! Large files fall back to a coarse diff to keep preview generation fast.

use crate::models::DiffLine;

const MAX_LCS_LINES: usize = 4000;

pub fn diff_lines(before: &str, after: &str) -> Vec<DiffLine> {
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();

    if before_lines.len() > MAX_LCS_LINES || after_lines.len() > MAX_LCS_LINES {
        return coarse_diff(&before_lines, &after_lines);
    }

    let mut table = vec![vec![0usize; after_lines.len() + 1]; before_lines.len() + 1];
    for i in (0..before_lines.len()).rev() {
        for j in (0..after_lines.len()).rev() {
            table[i][j] = if before_lines[i] == after_lines[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }

    let mut out = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < before_lines.len() && j < after_lines.len() {
        if before_lines[i] == after_lines[j] {
            out.push(DiffLine { kind: "context".into(), text: before_lines[i].to_string() });
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            out.push(DiffLine { kind: "del".into(), text: before_lines[i].to_string() });
            i += 1;
        } else {
            out.push(DiffLine { kind: "add".into(), text: after_lines[j].to_string() });
            j += 1;
        }
    }
    while i < before_lines.len() {
        out.push(DiffLine { kind: "del".into(), text: before_lines[i].to_string() });
        i += 1;
    }
    while j < after_lines.len() {
        out.push(DiffLine { kind: "add".into(), text: after_lines[j].to_string() });
        j += 1;
    }
    out
}

fn coarse_diff(before: &[&str], after: &[&str]) -> Vec<DiffLine> {
    let mut out = Vec::new();
    out.extend(before.iter().map(|line| DiffLine { kind: "del".into(), text: line.to_string() }));
    out.extend(after.iter().map(|line| DiffLine { kind: "add".into(), text: line.to_string() }));
    out
}

/// Trim a diff to a readable window around the changed lines.
pub fn compact_diff(lines: Vec<DiffLine>, context: usize) -> Vec<DiffLine> {
    let changed: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.kind != "context")
        .map(|(index, _)| index)
        .collect();
    if changed.is_empty() {
        return Vec::new();
    }
    let mut keep = vec![false; lines.len()];
    for index in changed {
        let start = index.saturating_sub(context);
        let end = (index + context + 1).min(lines.len());
        for flag in keep.iter_mut().take(end).skip(start) {
            *flag = true;
        }
    }
    let mut out = Vec::new();
    let mut skipped = false;
    for (index, line) in lines.into_iter().enumerate() {
        if keep[index] {
            if skipped {
                out.push(DiffLine { kind: "context".into(), text: "…".into() });
                skipped = false;
            }
            out.push(line);
        } else {
            skipped = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_input_produces_no_changes() {
        let lines = diff_lines("a\nb\n", "a\nb\n");
        assert!(lines.iter().all(|line| line.kind == "context"));
    }

    #[test]
    fn detects_added_and_removed_lines() {
        let lines = diff_lines("a\nb\n", "a\nc\n");
        assert!(lines.iter().any(|line| line.kind == "add" && line.text == "c"));
        assert!(lines.iter().any(|line| line.kind == "del" && line.text == "b"));
    }

    #[test]
    fn compact_diff_keeps_context() {
        let mut source = String::new();
        for i in 0..50 {
            source.push_str(&format!("line {i}\n"));
        }
        let mut target = source.clone();
        target = target.replace("line 25", "line 25 changed");
        let lines = compact_diff(diff_lines(&source, &target), 2);
        assert!(lines.len() < 20);
        assert!(lines.iter().any(|line| line.kind == "add"));
    }
}
