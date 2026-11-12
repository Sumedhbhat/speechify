//! Usage statistics: how much, how fast, where and when.

use crate::store::{Clock, Dictation, Outcome};
use serde::Serialize;
use std::collections::BTreeMap;

/// Average typing speed that dictation is compared with.
const TYPING_WPM: f64 = 40.0;
/// Weeks shown in the activity calendar.
const WEEKS: i64 = 26;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Day {
    /// YYYY-MM-DD in the user's time zone.
    pub date: String,
    pub words: u32,
    /// 0 none, then 1-249, 250-499, 500-749 and 750 or more.
    pub band: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Share {
    pub name: String,
    pub words: u32,
    pub dictations: u32,
    /// Fraction of all words, 0 to 1.
    pub share: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct Insights {
    pub words: u32,
    pub dictations: u32,
    pub minutes_spoken: f64,
    pub wpm: u32,
    /// Minutes saved compared with typing the same words.
    pub minutes_saved: u32,
    /// Corrections the formatter made: fillers, backtracks, dictionary fixes.
    pub fixes: u32,
    pub today_words: u32,
    pub week_words: u32,
    pub streak: u32,
    pub longest_streak: u32,
    pub active_days: u32,
    pub days: Vec<Day>,
    pub apps: Vec<Share>,
    pub categories: Vec<Share>,
    /// Words by hour of day, 0 to 23.
    pub hours: Vec<u32>,
    /// Words by weekday, Monday first.
    pub weekdays: Vec<u32>,
    /// Time from releasing the key to the text appearing.
    pub median_ms: u32,
    pub p95_ms: u32,
    /// Fraction of dictations the language model rewrote.
    pub rewritten_share: f64,
    /// Reserved for validated accuracy. Teacher agreement is not accuracy;
    /// keep this null for clients that still label it that way.
    pub accuracy: Option<f64>,
    /// 1 minus mean teacher disagreement, not ground-truth accuracy.
    pub teacher_agreement: Option<f64>,
    pub reviewed: u32,
}

fn local_day(d: &Dictation) -> i64 {
    (d.created_at + d.offset_min as i64 * 60).div_euclid(86_400)
}

/// Days since the epoch to a calendar date (Howard Hinnant's algorithm).
pub fn date(day: i64) -> String {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn band(words: u32) -> u8 {
    match words {
        0 => 0,
        1..=249 => 1,
        250..=499 => 2,
        500..=749 => 3,
        _ => 4,
    }
}

fn shares(groups: BTreeMap<String, (u32, u32)>, total: u32, limit: usize) -> Vec<Share> {
    let mut list: Vec<Share> = groups
        .into_iter()
        .map(|(name, (words, dictations))| Share {
            name,
            words,
            dictations,
            share: words as f64 / total.max(1) as f64,
        })
        .collect();
    list.sort_by(|a, b| b.words.cmp(&a.words).then(a.name.cmp(&b.name)));
    list.truncate(limit);
    list
}

fn percentile(sorted: &[u32], fraction: f64) -> u32 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[((sorted.len() - 1) as f64 * fraction).round() as usize]
}

pub fn compute(dictations: &[Dictation], clock: Clock) -> Insights {
    let today = (clock.now + clock.offset_min as i64 * 60).div_euclid(86_400);
    let useful: Vec<&Dictation> = dictations
        .iter()
        .filter(|d| matches!(d.outcome, Outcome::Pasted | Outcome::Copied) && d.words > 0)
        .collect();
    let mut out = Insights {
        hours: vec![0; 24],
        weekdays: vec![0; 7],
        ..Insights::default()
    };
    let mut per_day: BTreeMap<i64, u32> = BTreeMap::new();
    let (mut apps, mut categories) = (BTreeMap::new(), BTreeMap::new());
    let (mut speech_ms, mut timed_words, mut saved, mut rewritten) = (0u64, 0u64, 0f64, 0u32);
    let mut latencies: Vec<u32> = Vec::new();
    for d in &useful {
        let day = local_day(d);
        out.words += d.words;
        out.fixes += d.fixes;
        *per_day.entry(day).or_default() += d.words;
        if d.speech_ms > 0 {
            speech_ms += d.speech_ms as u64;
            timed_words += d.words as u64;
        }
        saved += d.words as f64 / TYPING_WPM - d.duration_ms as f64 / 60_000.0;
        rewritten += u32::from(d.rewritten);
        latencies.push(d.total_ms);
        let local_seconds = (d.created_at + d.offset_min as i64 * 60).rem_euclid(86_400);
        out.hours[(local_seconds / 3600) as usize] += d.words;
        // 1 January 1970 was a Thursday.
        out.weekdays[(day + 3).rem_euclid(7) as usize] += d.words;
        let app = if d.app.is_empty() { "Unknown".to_string() } else { d.app.clone() };
        let entry: &mut (u32, u32) = apps.entry(app).or_default();
        *entry = (entry.0 + d.words, entry.1 + 1);
        let entry: &mut (u32, u32) = categories.entry(d.category.label().to_string()).or_default();
        *entry = (entry.0 + d.words, entry.1 + 1);
    }
    out.dictations = useful.len() as u32;
    out.minutes_spoken = speech_ms as f64 / 60_000.0;
    out.wpm = if speech_ms > 0 { (timed_words as f64 / out.minutes_spoken).round() as u32 } else { 0 };
    out.minutes_saved = saved.max(0.0).round() as u32;
    out.today_words = per_day.get(&today).copied().unwrap_or(0);
    out.week_words = (today - 6..=today).filter_map(|day| per_day.get(&day)).sum();
    out.active_days = per_day.len() as u32;
    let mut run = 0;
    let mut previous = None;
    for day in per_day.keys() {
        run = if previous == Some(day - 1) { run + 1 } else { 1 };
        out.longest_streak = out.longest_streak.max(run);
        previous = Some(*day);
    }
    // Today not dictated yet does not break a streak that ran through yesterday.
    let anchor = if per_day.contains_key(&today) { today } else { today - 1 };
    out.streak = (0..).take_while(|back| per_day.contains_key(&(anchor - back))).count() as u32;
    // Start the calendar on a Monday so it draws as whole weeks.
    let first = today - (today + 3).rem_euclid(7) - (WEEKS - 1) * 7;
    out.days = (first..=today)
        .map(|day| {
            let words = per_day.get(&day).copied().unwrap_or(0);
            Day {
                date: date(day),
                words,
                band: band(words),
            }
        })
        .collect();
    out.apps = shares(apps, out.words, 12);
    out.categories = shares(categories, out.words, 8);
    latencies.sort_unstable();
    out.median_ms = percentile(&latencies, 0.5);
    out.p95_ms = percentile(&latencies, 0.95);
    out.rewritten_share = rewritten as f64 / useful.len().max(1) as f64;
    let scores: Vec<f64> = dictations.iter().filter_map(|d| d.teacher_wer).collect();
    out.reviewed = scores.len() as u32;
    out.teacher_agreement =
        (!scores.is_empty()).then(|| (1.0 - scores.iter().sum::<f64>() / scores.len() as f64).clamp(0.0, 1.0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Category;
    use crate::testing::dictation;

    // Wednesday 14 January 2026, 10:00 in India.
    const DAY: i64 = 20_467;
    const NOW: Clock = Clock {
        now: DAY * 86_400 + 4 * 3600 + 1800,
        offset_min: 330,
    };

    fn at(days_ago: i64, text: &str, app: &str) -> Dictation {
        dictation(text, app, NOW.now - days_ago * 86_400)
    }

    #[test]
    fn calendar_dates() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(DAY), "2026-01-14");
        assert_eq!(date(19_782), "2024-02-29");
        assert_eq!(date(-1), "1969-12-31");
    }

    #[test]
    fn empty_history() {
        let insights = compute(&[], NOW);
        assert_eq!(
            (insights.words, insights.wpm, insights.streak, insights.median_ms),
            (0, 0, 0, 0)
        );
        assert_eq!(insights.accuracy, None);
        assert_eq!(insights.days.len(), 25 * 7 + 3);
        assert_eq!(insights.days.last().unwrap().date, "2026-01-14");
        assert!(insights.apps.is_empty());
    }

    #[test]
    fn totals_speed_and_time_saved() {
        let mut slow = at(0, "one two three four five six seven eight nine ten", "Slack");
        slow.fixes = 3;
        slow.rewritten = true;
        slow.total_ms = 900;
        let cancelled = Dictation {
            outcome: Outcome::Cancelled,
            ..at(0, "ignored words here", "Slack")
        };
        let untimed = Dictation {
            speech_ms: 0,
            ..at(0, "two words", "")
        };
        let insights = compute(&[slow, cancelled, untimed], NOW);
        assert_eq!(insights.words, 12);
        assert_eq!(insights.dictations, 2);
        assert_eq!(insights.wpm, 120);
        assert_eq!(insights.fixes, 3);
        // 12 words is 18 s of typing at 40 wpm; speaking took 7.2 s.
        assert_eq!(insights.minutes_saved, 0);
        assert_eq!(insights.rewritten_share, 0.5);
        assert_eq!((insights.median_ms, insights.p95_ms), (900, 900));
        assert_eq!(insights.apps[1].name, "Unknown");
        let long = Dictation {
            words: 400,
            duration_ms: 120_000,
            ..at(0, "x", "Docs")
        };
        assert_eq!(compute(&[long], NOW).minutes_saved, 8);
    }

    #[test]
    fn streaks_weeks_and_bands() {
        let mut rows = vec![
            at(1, "a b", "A"),
            at(2, "a", "A"),
            at(3, "a", "A"),
            at(5, "a", "A"),
            at(9, "a", "A"),
        ];
        rows.push(Dictation {
            words: 300,
            ..at(6, "x", "A")
        });
        rows.push(Dictation {
            words: 600,
            ..at(7, "x", "A")
        });
        rows.push(Dictation {
            words: 800,
            ..at(8, "x", "A")
        });
        let insights = compute(&rows, NOW);
        assert_eq!(insights.today_words, 0);
        assert_eq!(insights.streak, 3);
        assert_eq!(insights.longest_streak, 5);
        assert_eq!(insights.active_days, 8);
        assert_eq!(insights.week_words, 2 + 1 + 1 + 1 + 300);
        let bands: Vec<u8> = insights.days.iter().rev().take(10).map(|d| d.band).collect();
        assert_eq!(bands, vec![0, 1, 1, 1, 0, 1, 2, 3, 4, 1]);
        rows.push(at(0, "today", "A"));
        assert_eq!(compute(&rows, NOW).streak, 4);
        assert_eq!(compute(&[at(4, "old", "A")], NOW).streak, 0);
    }

    #[test]
    fn breakdown_by_app_category_hour_and_weekday() {
        let mut email = at(0, "a b c d e f", "Outlook");
        email.category = Category::Email;
        let mut chat = at(2, "a b", "Slack");
        chat.category = Category::Work;
        // 23:30 UTC the day before is 05:00 in India on the same local day.
        let early = Dictation {
            created_at: (DAY - 1) * 86_400 + 23 * 3600 + 1800,
            ..at(0, "a b", "Slack")
        };
        let insights = compute(&[email, chat, early], NOW);
        assert_eq!(
            insights
                .apps
                .iter()
                .map(|s| (s.name.as_str(), s.words, s.dictations))
                .collect::<Vec<_>>(),
            vec![("Outlook", 6, 1), ("Slack", 4, 2)]
        );
        assert_eq!(insights.apps[0].share, 0.6);
        assert_eq!(
            insights.categories.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            vec!["Email", "Other", "Work messages"]
        );
        assert_eq!(insights.hours[10], 6 + 2);
        assert_eq!(insights.hours[5], 2);
        assert_eq!(insights.weekdays, vec![2, 0, 8, 0, 0, 0, 0]);
        assert_eq!(insights.today_words, 8);
    }

    #[test]
    fn teacher_agreement_is_never_reported_as_true_accuracy() {
        let a = Dictation {
            teacher_wer: Some(0.1),
            ..at(0, "a", "A")
        };
        let b = Dictation {
            teacher_wer: Some(0.3),
            ..at(0, "b", "A")
        };
        let c = Dictation {
            teacher_wer: Some(4.0),
            ..at(0, "c", "A")
        };
        let insights = compute(&[a.clone(), b, at(0, "c", "A")], NOW);
        assert_eq!(insights.reviewed, 2);
        assert!((insights.teacher_agreement.unwrap() - 0.8).abs() < 1e-9);
        assert_eq!(compute(&[a, c], NOW).teacher_agreement, Some(0.0));
        assert_eq!(insights.accuracy, None);
        let json = serde_json::to_value(insights).unwrap();
        assert!(json["accuracy"].is_null());
        assert_eq!(json["teacher_agreement"], 0.8);
    }
}
