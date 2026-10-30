//! Learned word rules, vocabulary hints and learning status.
use super::Store;
use crate::config::{err, Result};
use serde_json::{json, Value};

impl Store {
    pub fn learning(&self) -> Result<Value> {
        let rules = self
            .db
            .prepare("SELECT wrong, right, count FROM rules ORDER BY count DESC")
            .map_err(err)?
            .query_map([], |row| {
                Ok(json!({
                    "wrong": row.get::<_, String>(0)?,
                    "right": row.get::<_, String>(1)?,
                    "count": row.get::<_, i64>(2)?,
                }))
            })
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        let vocab = self
            .db
            .prepare("SELECT term,count FROM vocab ORDER BY count DESC")
            .map_err(err)?
            .query_map([], |r| {
                Ok(json!({"term":r.get::<_,String>(0)?,"count":r.get::<_,i64>(1)?}))
            })
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        Ok(json!({
            "rules": rules,
            "vocab": vocab,
            "versions": self.get_state("versions").unwrap_or(json!([])),
            "teacher": self.get_state("teacher_status").unwrap_or(json!("Idle")),
            "training": self.get_state("training_status").unwrap_or(json!("Not started")),
        }))
    }
    pub fn forget(&self, kind: &str, key: &str) -> Result<()> {
        match kind {
            "rule" => {
                self.db
                    .execute("DELETE FROM rules WHERE wrong=?", [key])
                    .map_err(err)?;
            }
            "vocab" => {
                self.db
                    .execute("DELETE FROM vocab WHERE term=?", [key])
                    .map_err(err)?;
            }
            _ => return Err("Unknown learning item".into()),
        }
        Ok(())
    }
    pub fn apply(&self, text: &str, threshold: u64) -> Result<String> {
        let mut rules=self.db.prepare("SELECT wrong,right FROM rules WHERE count>=? ORDER BY length(wrong) DESC,count DESC").map_err(err)?;
        let rows = rules
            .query_map([threshold as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        if rows.is_empty() {
            return Ok(text.into());
        }
        let mut seen = std::collections::HashSet::new();
        let mut patterns = Vec::new();
        let mut replacements = Vec::new();
        for (a, b) in rows {
            if !seen.insert(a.clone()) {
                continue;
            }
            patterns.push(format!("({})", regex::escape(&a).replace(' ', r"\s+")));
            replacements.push(b);
        }
        let re = regex::Regex::new(&format!(r"(?i)\b(?:{})\b", patterns.join("|"))).map_err(err)?;
        Ok(re
            .replace_all(text, |cap: &regex::Captures| {
                let rule = cap
                    .iter()
                    .skip(1)
                    .position(|group| group.is_some())
                    .unwrap();
                replacements[rule].as_str()
            })
            .into_owned())
    }
    pub fn prompt(&self) -> Result<String> {
        let terms = self
            .db
            .prepare("SELECT term FROM vocab ORDER BY count DESC,term")
            .map_err(err)?
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        let mut p = String::new();
        for t in terms {
            if p.len() + t.len() + if p.is_empty() { 0 } else { 2 } > 450 {
                break;
            }
            if !p.is_empty() {
                p.push_str(", ");
            }
            p.push_str(&t);
        }
        Ok(p)
    }
}
