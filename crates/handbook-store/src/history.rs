//! Capture history of one watched source and line diffs between its
//! consecutive captures (SA-14 "source changed").
//!
//! The watch sweep keeps every changed capture as a new row, so a
//! source's history is its `capture` rows in id order. Text sidecars
//! (`text/<id>.txt`) hold one line per block element, so a line diff
//! of two sidecars is the reader-visible change. Two consecutive
//! captures with the same `sha256` are the same bytes read again by a
//! newer text extractor (P360): their diff is extractor drift, never a
//! change on the page.

use rusqlite::{Connection, OptionalExtension};

use contracts::Result;

use crate::store::Store;

/// One capture row of a source's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCapture {
    pub id: i64,
    pub source_id: i64,
    pub captured_at: i64,
    pub kind: String,
    pub trust: String,
    pub sha256: String,
    pub bytes_raw: i64,
}

const CAPTURE_COLUMNS: &str = "id, source_id, captured_at, kind, trust, sha256, bytes_raw";

fn capture_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SourceCapture> {
    Ok(SourceCapture {
        id: row.get(0)?,
        source_id: row.get(1)?,
        captured_at: row.get(2)?,
        kind: row.get(3)?,
        trust: row.get(4)?,
        sha256: row.get(5)?,
        bytes_raw: row.get(6)?,
    })
}

/// A source's captures, newest first, capped at `limit`.
pub fn source_captures(
    conn: &Connection,
    source_id: i64,
    limit: i64,
) -> Result<Vec<SourceCapture>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {CAPTURE_COLUMNS} FROM capture
         WHERE source_id = ?1 ORDER BY id DESC LIMIT ?2"
    ))?;
    let rows = stmt
        .query_map(rusqlite::params![source_id, limit], capture_row)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// One capture of a source, or `None` for an unknown or unattributed id.
pub fn capture(conn: &Connection, capture_id: i64) -> Result<Option<SourceCapture>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {CAPTURE_COLUMNS} FROM capture
                 WHERE id = ?1 AND source_id IS NOT NULL"
            ),
            [capture_id],
            capture_row,
        )
        .optional()?)
}

/// The capture of the same source and kind just before `later`
/// (rejected captures are not history), or `None` for the first one.
pub fn previous_capture(conn: &Connection, later: &SourceCapture) -> Result<Option<SourceCapture>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {CAPTURE_COLUMNS} FROM capture
                 WHERE source_id = ?1 AND kind = ?2 AND id < ?3 AND trust != 'rejected'
                 ORDER BY id DESC LIMIT 1"
            ),
            rusqlite::params![later.source_id, later.kind, later.id],
            capture_row,
        )
        .optional()?)
}

/// A capture's text sidecar, or `None` when the file is missing.
pub fn sidecar_text(store: &Store, capture_id: i64) -> Option<String> {
    std::fs::read_to_string(store.root().join("text").join(format!("{capture_id}.txt"))).ok()
}

/// One line of a diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineOp {
    Same,
    Added,
    Removed,
}

impl LineOp {
    pub fn as_str(self) -> &'static str {
        match self {
            LineOp::Same => "same",
            LineOp::Added => "added",
            LineOp::Removed => "removed",
        }
    }
}

/// One diff line with its 0-based line number on each side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub op: LineOp,
    pub text: String,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
}

/// Cap on the LCS table (cells) after trimming the shared head and
/// tail; past it the middle diffs by line membership instead.
const MAX_LCS_CELLS: usize = 2_000_000;

/// Sidecar lines exactly as stored (`\n`-joined, no trailing newline),
/// so line numbers map back to byte offsets.
fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').collect()
    }
}

/// Line diff of two sidecars, in new-text order with removals placed
/// before the line that replaced them. Shared head and tail lines are
/// trimmed first (a banner added to a long page costs nothing); the
/// middle is an LCS diff, or a membership diff when it is huge.
pub fn line_diff(old: &str, new: &str) -> Vec<DiffLine> {
    let a = split_lines(old);
    let b = split_lines(new);
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a[head..]
        .iter()
        .rev()
        .zip(b[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (a_mid, b_mid) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let same = |i: usize, j: usize, text: &str| DiffLine {
        op: LineOp::Same,
        text: text.to_string(),
        old_line: Some(i),
        new_line: Some(j),
    };
    let mut out: Vec<DiffLine> = (0..head).map(|i| same(i, i, b[i])).collect();
    let pairs = if a_mid.len().saturating_mul(b_mid.len()) <= MAX_LCS_CELLS {
        lcs_pairs(a_mid, b_mid)
    } else {
        membership_pairs(a_mid, b_mid)
    };
    let (mut i, mut j) = (0, 0);
    for (pi, pj) in pairs.into_iter().chain([(a_mid.len(), b_mid.len())]) {
        out.extend(a_mid[i..pi].iter().zip(i..).map(|(text, k)| DiffLine {
            op: LineOp::Removed,
            text: text.to_string(),
            old_line: Some(head + k),
            new_line: None,
        }));
        out.extend(b_mid[j..pj].iter().zip(j..).map(|(text, k)| DiffLine {
            op: LineOp::Added,
            text: text.to_string(),
            old_line: None,
            new_line: Some(head + k),
        }));
        if pi < a_mid.len() {
            out.push(same(head + pi, head + pj, b_mid[pj]));
        }
        (i, j) = (pi + 1, pj + 1);
    }
    let (a_tail, b_tail) = (a.len() - tail, b.len() - tail);
    out.extend((0..tail).map(|k| same(a_tail + k, b_tail + k, b[b_tail + k])));
    out
}

/// Matched `(old, new)` index pairs of a longest common subsequence.
fn lcs_pairs(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    // lens[i][j] = LCS length of a[i..] and b[j..].
    let mut lens = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lens[at(i, j)] = if a[i] == b[j] {
                lens[at(i + 1, j + 1)] + 1
            } else {
                lens[at(i + 1, j)].max(lens[at(i, j + 1)])
            };
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if lens[at(i + 1, j)] >= lens[at(i, j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// Fallback for huge middles: pair equal lines greedily in order.
fn membership_pairs(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    let mut from = 0;
    for (j, line) in b.iter().enumerate() {
        if let Some(k) = a[from..].iter().position(|x| x == line) {
            pairs.push((from + k, j));
            from += k + 1;
        }
    }
    pairs
}

/// The diff between two captures of one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureDiff {
    pub from: SourceCapture,
    pub to: SourceCapture,
    /// Same bytes, re-read by a newer extractor (P360): no page change.
    pub reread: bool,
    /// A sidecar is missing, so the diff is against empty text.
    pub missing_sidecar: bool,
    pub lines: Vec<DiffLine>,
}

impl CaptureDiff {
    pub fn added(&self) -> impl Iterator<Item = &DiffLine> {
        self.lines.iter().filter(|l| l.op == LineOp::Added)
    }

    pub fn removed(&self) -> impl Iterator<Item = &DiffLine> {
        self.lines.iter().filter(|l| l.op == LineOp::Removed)
    }
}

/// Diff a capture against the one before it on the same source, or
/// `None` when it is unattributed, unknown, or the source's first.
pub fn diff_with_previous(
    conn: &Connection,
    store: &Store,
    capture_id: i64,
) -> Result<Option<CaptureDiff>> {
    let Some(to) = capture(conn, capture_id)? else {
        return Ok(None);
    };
    let Some(from) = previous_capture(conn, &to)? else {
        return Ok(None);
    };
    let old = sidecar_text(store, from.id);
    let new = sidecar_text(store, to.id);
    let missing_sidecar = old.is_none() || new.is_none();
    let lines = line_diff(old.as_deref().unwrap_or(""), new.as_deref().unwrap_or(""));
    Ok(Some(CaptureDiff {
        reread: from.sha256 == to.sha256,
        from,
        to,
        missing_sidecar,
        lines,
    }))
}

/// A contiguous run of lines in the newer sidecar: added lines plus
/// their context, with the byte span they occupy in that sidecar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub first_line: usize,
    /// Exclusive.
    pub end_line: usize,
    pub byte_start: usize,
    /// Exclusive.
    pub byte_end: usize,
}

/// Runs of added lines in `new`, each widened by `context` lines on
/// both sides and merged when they touch. Byte spans index `new`, so
/// a quote inside a hunk is a quote inside the newer sidecar.
pub fn added_hunks(lines: &[DiffLine], new: &str, context: usize) -> Vec<Hunk> {
    let rows = split_lines(new);
    let mut offsets = Vec::with_capacity(rows.len() + 1);
    let mut at = 0;
    for row in &rows {
        offsets.push(at);
        at += row.len() + 1;
    }
    let mut hunks: Vec<Hunk> = Vec::new();
    for line in lines.iter().filter(|l| l.op == LineOp::Added) {
        let Some(n) = line.new_line.filter(|n| *n < rows.len()) else {
            continue;
        };
        let first = n.saturating_sub(context);
        let end = (n + 1 + context).min(rows.len());
        match hunks.last_mut() {
            Some(last) if first <= last.end_line => last.end_line = last.end_line.max(end),
            _ => hunks.push(Hunk {
                first_line: first,
                end_line: end,
                byte_start: 0,
                byte_end: 0,
            }),
        }
    }
    for hunk in &mut hunks {
        hunk.byte_start = offsets[hunk.first_line];
        let last = hunk.end_line - 1;
        hunk.byte_end = offsets[last] + rows[last].len();
    }
    hunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops(lines: &[DiffLine]) -> Vec<(char, &str)> {
        lines
            .iter()
            .map(|l| {
                let c = match l.op {
                    LineOp::Same => ' ',
                    LineOp::Added => '+',
                    LineOp::Removed => '-',
                };
                (c, l.text.as_str())
            })
            .collect()
    }

    #[test]
    fn banner_added_to_a_page_is_one_added_line() {
        let old = "Apostilles\nMail to PO Box 40220\nFee $15";
        let new = "Apostilles\nApostille services are unavailable\nMail to PO Box 40220\nFee $15";
        let diff = line_diff(old, new);
        assert_eq!(
            ops(&diff),
            vec![
                (' ', "Apostilles"),
                ('+', "Apostille services are unavailable"),
                (' ', "Mail to PO Box 40220"),
                (' ', "Fee $15"),
            ]
        );
        assert_eq!(diff[1].new_line, Some(1));
        assert_eq!(diff[2].old_line, Some(1));
        assert_eq!(diff[2].new_line, Some(2));
    }

    #[test]
    fn replaced_line_is_removed_then_added() {
        let diff = line_diff("a\nFee $15\nz", "a\nFee $20\nz");
        assert_eq!(
            ops(&diff),
            vec![(' ', "a"), ('-', "Fee $15"), ('+', "Fee $20"), (' ', "z")]
        );
    }

    #[test]
    fn empty_sides_and_identical_texts() {
        assert!(line_diff("", "").is_empty());
        assert_eq!(ops(&line_diff("", "x\ny")), vec![('+', "x"), ('+', "y")]);
        assert_eq!(ops(&line_diff("x", "")), vec![('-', "x")]);
        assert!(
            line_diff("a\nb", "a\nb")
                .iter()
                .all(|l| l.op == LineOp::Same)
        );
    }

    #[test]
    fn huge_middles_fall_back_to_membership() {
        let old: Vec<String> = (0..2_000).map(|i| format!("old {i}")).collect();
        let mut new = old.clone();
        new.reverse();
        new.insert(1_000, "NOTICE".to_string());
        let diff = line_diff(&old.join("\n"), &new.join("\n"));
        assert_eq!(
            diff.iter()
                .filter(|l| l.op == LineOp::Added && l.text == "NOTICE")
                .count(),
            1
        );
        // Every line of both sides appears exactly once.
        let kept = diff.iter().filter(|l| l.op != LineOp::Removed).count();
        let dropped = diff.iter().filter(|l| l.op != LineOp::Added).count();
        assert_eq!(kept, new.len());
        assert_eq!(dropped, old.len());
    }

    #[test]
    fn hunks_carry_context_and_exact_byte_spans() {
        let old = "h1\np1\np2\np3\np4\np5";
        let new = "h1\nNOTICE one\np1\np2\np3\np4\nNOTICE two\np5";
        let diff = line_diff(old, new);
        let hunks = added_hunks(&diff, new, 1);
        assert_eq!(hunks.len(), 2);
        assert_eq!(
            &new[hunks[0].byte_start..hunks[0].byte_end],
            "h1\nNOTICE one\np1"
        );
        assert_eq!(
            &new[hunks[1].byte_start..hunks[1].byte_end],
            "p4\nNOTICE two\np5"
        );
        // Touching runs merge into one hunk.
        let merged = added_hunks(&diff, new, 3);
        assert_eq!(merged.len(), 1);
        assert_eq!(&new[merged[0].byte_start..merged[0].byte_end], new);
        assert!(added_hunks(&line_diff(new, new), new, 1).is_empty());
    }
}
