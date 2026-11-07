use chrono::{DateTime, Duration, Local, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Style {
    Popup,
    #[default]
    Island,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub enabled: bool,
    pub style: Style,
    pub weekly_timed: bool,
    pub weekly_bonus: bool,
    pub hourly_timed: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            style: Style::Island,
            weekly_timed: true,
            weekly_bonus: true,
            hourly_timed: true,
        }
    }
}

#[derive(Clone)]
pub struct Notice {
    pub title: String,
    pub body: String,
    pub important: bool,
    pub task: bool,
}

fn reset_at(window: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(window.get("resetsAt")?.as_str()?)
        .ok()
        .map(|date| date.with_timezone(&Utc))
}

fn remaining(window: &Value) -> Option<f64> {
    window
        .get("remainingPercent")?
        .as_f64()
        .filter(|value| value.is_finite())
}

fn timed_reset(old: &Value, new: &Value, now: DateTime<Utc>, max_delay: Duration) -> bool {
    let (Some(old_at), Some(new_at)) = (reset_at(old), reset_at(new)) else {
        return false;
    };
    let duration = old
        .get("windowDurationMins")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    duration > 0
        && new_at - old_at >= Duration::minutes(duration / 2)
        && old_at <= now + Duration::minutes(2)
        && now - old_at <= max_delay
        && new_at > now
}

fn next_reset(window: &Value) -> String {
    reset_at(window)
        .map(|date| date.with_timezone(&Local).format("%m/%d %H:%M").to_string())
        .unwrap_or_else(|| "--".into())
}

fn percent(window: &Value) -> String {
    remaining(window)
        .map(|value| format!("{:.0}%", value.clamp(0.0, 100.0)))
        .unwrap_or_else(|| "--".into())
}

pub fn detect(
    previous: Option<&Value>,
    current: &Value,
    settings: &Settings,
    now: DateTime<Utc>,
) -> Vec<Notice> {
    if !settings.enabled {
        return Vec::new();
    }
    let Some(previous) = previous else {
        return Vec::new();
    };
    let previous_at = previous
        .get("fetchedAt")
        .and_then(Value::as_str)
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|date| date.with_timezone(&Utc));
    if previous_at.is_none_or(|date| date > now || now - date > Duration::days(8)) {
        return Vec::new();
    }

    let mut notices = Vec::new();
    if let (Some(old), Some(new)) = (previous.get("secondary"), current.get("secondary")) {
        if settings.weekly_timed && timed_reset(old, new, now, Duration::hours(12)) {
            notices.push(Notice {
                title: "QuotaDock · 7 天额度已重置".into(),
                body: format!("当前剩余 {}，下次重置 {}。", percent(new), next_reset(new)),
                important: true,
                task: false,
            });
        } else if settings.weekly_bonus {
            let early = reset_at(old).is_some_and(|date| date > now + Duration::minutes(5));
            let recent = previous_at.is_some_and(|date| now - date <= Duration::hours(24));
            if let (Some(before), Some(after)) = (remaining(old), remaining(new)) {
                if early && recent && after - before >= 5.0 {
                    notices.push(Notice {
                        title: "QuotaDock · 7 天额度提前恢复".into(),
                        body: format!(
                            "剩余额度由 {:.0}% 升至 {:.0}%，可能是官方奖励或额度调整。",
                            before, after
                        ),
                        important: true,
                        task: false,
                    });
                }
            }
        }
    }
    if settings.hourly_timed {
        if let (Some(old), Some(new)) = (previous.get("primary"), current.get("primary")) {
            if timed_reset(old, new, now, Duration::hours(2)) {
                notices.push(Notice {
                    title: "QuotaDock · 5 小时额度已重置".into(),
                    body: format!("当前剩余 {}，下次重置 {}。", percent(new), next_reset(new)),
                    important: false,
                    task: false,
                });
            }
        }
    }
    notices
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn old_settings_default_to_island_and_style_choice_round_trips() {
        let old: Settings =
            serde_json::from_value(json!({"enabled":true,"weeklyTimed":false})).unwrap();
        assert!(old.style == Style::Island);
        assert!(!old.weekly_timed);
        let chosen = Settings {
            style: Style::Popup,
            ..Settings::default()
        };
        let loaded: Settings =
            serde_json::from_value(serde_json::to_value(chosen).unwrap()).unwrap();
        assert!(loaded.style == Style::Popup);
    }

    fn snapshot(
        at: DateTime<Utc>,
        short: (i64, DateTime<Utc>),
        long: (i64, DateTime<Utc>),
    ) -> Value {
        json!({
            "fetchedAt": at.to_rfc3339(),
            "primary": {"remainingPercent": short.0, "windowDurationMins": 300, "resetsAt": short.1.to_rfc3339()},
            "secondary": {"remainingPercent": long.0, "windowDurationMins": 10080, "resetsAt": long.1.to_rfc3339()}
        })
    }

    #[test]
    fn scheduled_resets_show_weekly_before_hourly() {
        let now = Utc::now();
        let old = snapshot(
            now - Duration::minutes(3),
            (40, now - Duration::minutes(1)),
            (20, now - Duration::minutes(1)),
        );
        let new = snapshot(
            now,
            (100, now + Duration::hours(5)),
            (100, now + Duration::days(7)),
        );
        let notices = detect(Some(&old), &new, &Settings::default(), now);
        assert_eq!(notices.len(), 2);
        assert!(notices[0].title.contains("7 天"));
        assert!(notices[0].important);
        assert!(notices[1].title.contains("5 小时"));
        assert!(!notices[1].important);
        assert!(detect(Some(&new), &new, &Settings::default(), now).is_empty());
    }

    #[test]
    fn early_weekly_increase_is_labeled_as_possible_reward() {
        let now = Utc::now();
        let reset = now + Duration::days(2);
        let old = snapshot(
            now - Duration::minutes(3),
            (50, now + Duration::hours(3)),
            (60, reset),
        );
        let new = snapshot(now, (50, now + Duration::hours(3)), (68, reset));
        let notices = detect(Some(&old), &new, &Settings::default(), now);
        assert_eq!(notices.len(), 1);
        assert!(notices[0].body.contains("可能是官方奖励"));
    }

    #[test]
    fn disabled_and_first_read_do_not_popup() {
        let now = Utc::now();
        let old = snapshot(
            now - Duration::minutes(3),
            (40, now - Duration::minutes(1)),
            (20, now - Duration::minutes(1)),
        );
        let new = snapshot(
            now,
            (100, now + Duration::hours(5)),
            (100, now + Duration::days(7)),
        );
        assert!(detect(None, &new, &Settings::default(), now).is_empty());
        let settings = Settings {
            enabled: false,
            ..Settings::default()
        };
        assert!(detect(Some(&old), &new, &settings, now).is_empty());
    }
}
