//! Pure label selection and bounded word-difference extraction.
use serde_json::Value;

pub fn label(row: &Value, min: f64) -> Option<(String, &'static str)> {
    if row["excluded"].as_i64().unwrap_or(0) != 0 || !row["audio"].is_string() {
        return None;
    }
    if let Some(t) = row["corrected"].as_str().filter(|t| !t.trim().is_empty()) {
        return Some((t.into(), "corrected"));
    }
    if row["teacher_note"].is_null() && row["teacher_confidence"].as_f64().unwrap_or(0.) >= min {
        if let Some(t) = row["teacher_text"]
            .as_str()
            .filter(|t| !t.trim().is_empty())
        {
            return Some((t.into(), "teacher"));
        }
    }
    None
}
pub fn edits(a: &str, b: &str) -> Vec<(String, String)> {
    // LCS anchors separate bounded replacements from rewrites and insertions.
    let aa: Vec<_> = a.split_whitespace().collect();
    let bb: Vec<_> = b.split_whitespace().collect();
    if aa.len() > 512 || bb.len() > 512 {
        return vec![];
    }
    let norm = |s: &str| {
        s.trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase()
    };
    let mut dp = vec![vec![0; bb.len() + 1]; aa.len() + 1];
    for i in (0..aa.len()).rev() {
        for j in (0..bb.len()).rev() {
            dp[i][j] = if norm(aa[i]) == norm(bb[j]) {
                1 + dp[i + 1][j + 1]
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = vec![];
    while i < aa.len() && j < bb.len() {
        if norm(aa[i]) == norm(bb[j]) {
            let x = aa[i].trim_matches(|c: char| !c.is_alphanumeric());
            let y = bb[j].trim_matches(|c: char| !c.is_alphanumeric());
            if x != y {
                out.push((norm(x), y.into()));
            }
            i += 1;
            j += 1;
        } else {
            let (si, sj) = (i, j);
            while i < aa.len() && j < bb.len() && norm(aa[i]) != norm(bb[j]) {
                if dp[i + 1][j] >= dp[i][j + 1] {
                    i += 1;
                } else {
                    j += 1;
                }
            }
            if i == aa.len() {
                j = bb.len();
            }
            if j == bb.len() {
                i = aa.len();
            }
            if i > si && j > sj && i - si <= 4 && j - sj <= 4 {
                out.push((
                    aa[si..i]
                        .iter()
                        .map(|s| norm(s))
                        .collect::<Vec<_>>()
                        .join(" "),
                    bb[sj..j]
                        .join(" ")
                        .trim_matches(|c: char| !c.is_alphanumeric())
                        .into(),
                ));
            }
        }
    }
    out
}
