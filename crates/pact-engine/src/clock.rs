//! 时钟抽象（D8）：Clock trait + 生产/测试实现。
//! 引擎构造时注入；pact-core 不感知时钟（纯度约束 §2.1-3）。

use pact_core::Timestamp;
use std::time::{SystemTime, UNIX_EPOCH};

pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// 生产时钟：系统 UTC 时间，ISO-8601 秒精度。
/// 零 chrono/time 依赖——civil-from-days 手算（Howard Hinnant 算法）。
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        iso8601_utc(secs)
    }
}

/// Unix 秒 → ISO-8601 UTC（civil_from_days，覆盖 1970–9999 年足够）。
pub fn iso8601_utc(unix_secs: u64) -> String {
    let days = (unix_secs / 86400) as i64;
    let sod = unix_secs % 86400;
    let (h, mi, s) = (sod / 3600, (sod % 3600) / 60, sod % 60);

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let mo = if mp < 10 { mp + 3 } else { mp - 9 }; // 月份 [1, 12]
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// 测试时钟：单调递增计数（"T0"、"T1"、…），确定性重放（NFR3）。
pub struct LogicalClock(std::sync::atomic::AtomicU64);

impl LogicalClock {
    pub fn new() -> Self {
        Self(std::sync::atomic::AtomicU64::new(0))
    }
    pub fn new_at(start: u64) -> Self {
        Self(std::sync::atomic::AtomicU64::new(start))
    }
}

impl Default for LogicalClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for LogicalClock {
    fn now(&self) -> Timestamp {
        let n = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        format!("T{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_known_epochs() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601_utc(86_400), "1970-01-02T00:00:00Z");
        assert_eq!(iso8601_utc(946_684_800), "2000-01-01T00:00:00Z");
        // 闰日
        assert_eq!(iso8601_utc(1_709_164_800), "2024-02-29T00:00:00Z");
        assert_eq!(iso8601_utc(1_709_251_200), "2024-03-01T00:00:00Z");
        assert_eq!(iso8601_utc(1_767_225_600), "2026-01-01T00:00:00Z");
    }

    #[test]
    fn logical_clock_monotonic() {
        let c = LogicalClock::new();
        assert_eq!(c.now(), "T0");
        assert_eq!(c.now(), "T1");
        assert_eq!(c.now(), "T2");
    }
}
