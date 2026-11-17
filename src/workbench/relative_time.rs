//! "3 分钟前": how long ago a commit was, with VS Code's `fromNow` thresholds.

const MINUTE: i64 = 60;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;
const WEEK: i64 = 7 * DAY;
const MONTH: i64 = 30 * DAY;
const YEAR: i64 = 365 * DAY;

/// Describes `then` relative to `now`, both in seconds since the epoch. Under
/// 30 seconds — or a little in the future, from clock skew — is 刚刚.
pub fn relative_time(then: i64, now: i64) -> String {
    let seconds = now - then;
    if seconds < 30 {
        return "刚刚".to_owned();
    }
    let (count, unit) = if seconds < MINUTE {
        (seconds, "秒")
    } else if seconds < HOUR {
        (seconds / MINUTE, "分钟")
    } else if seconds < DAY {
        (seconds / HOUR, "小时")
    } else if seconds < WEEK {
        (seconds / DAY, "天")
    } else if seconds < MONTH {
        (seconds / WEEK, "周")
    } else if seconds < YEAR {
        (seconds / MONTH, "个月")
    } else {
        (seconds / YEAR, "年")
    };
    format!("{count} {unit}前")
}

/// Seconds since the epoch.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_match_vs_code() {
        let now = 1_700_000_000;
        let ago = |seconds| relative_time(now - seconds, now);
        assert_eq!(ago(-10), "刚刚");
        assert_eq!(ago(29), "刚刚");
        assert_eq!(ago(30), "30 秒前");
        assert_eq!(ago(59), "59 秒前");
        assert_eq!(ago(MINUTE), "1 分钟前");
        assert_eq!(ago(59 * MINUTE + 59), "59 分钟前");
        assert_eq!(ago(HOUR), "1 小时前");
        assert_eq!(ago(14 * HOUR), "14 小时前");
        assert_eq!(ago(6 * DAY), "6 天前");
        assert_eq!(ago(WEEK), "1 周前");
        assert_eq!(ago(29 * DAY), "4 周前");
        assert_eq!(ago(30 * DAY), "1 个月前");
        assert_eq!(ago(364 * DAY), "12 个月前");
        assert_eq!(ago(YEAR), "1 年前");
    }
}
