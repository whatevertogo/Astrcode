//! # 时间序列化辅助
//!
//! 统一处理“内部存 UTC、对用户展示本地时区”的时间策略。
//!
//! ## 设计原因
//!
//! - 运行时内部仍然使用 `DateTime<Utc>`，避免排序和比较语义分裂
//! - 持久化 JSONL、HTTP DTO 和前端回放需要与用户机器本地时间一致
//! - 反序列化时接受任意 RFC3339 offset，并统一转回 UTC 保存

use chrono::{DateTime, FixedOffset, Local, SecondsFormat, Utc};
use serde::{Deserialize, Deserializer, Serializer};

/// 将 UTC 时间格式化为本地时区 RFC3339。
pub fn format_local_rfc3339(value: DateTime<Utc>) -> String {
    value
        .with_timezone(&Local)
        .to_rfc3339_opts(SecondsFormat::Nanos, false)
}

/// 将可选 UTC 时间格式化为本地时区 RFC3339。
pub fn format_local_rfc3339_opt(value: Option<&DateTime<Utc>>) -> Option<String> {
    value.copied().map(format_local_rfc3339)
}

fn deserialize_localized_datetime<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    let localized = DateTime::<FixedOffset>::deserialize(deserializer)?;
    Ok(localized.with_timezone(&Utc))
}

/// `DateTime<Utc>` 的本地时区 RFC3339 serde 适配器。
pub mod local_rfc3339 {
    use super::*;

    pub fn serialize<S>(value: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format_local_rfc3339(*value))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_localized_datetime(deserializer)
    }
}

/// `Option<DateTime<Utc>>` 的本地时区 RFC3339 serde 适配器。
pub mod local_rfc3339_option {
    use super::*;

    pub fn serialize<S>(value: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(value) => serializer.serialize_some(&format_local_rfc3339(*value)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let localized = Option::<DateTime<FixedOffset>>::deserialize(deserializer)?;
        Ok(localized.map(|value| value.with_timezone(&Utc)))
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde::{Deserialize, Serialize};

    use super::*;

    #[test]
    fn format_local_rfc3339_uses_local_timezone_offset() {
        let timestamp = Utc
            .with_ymd_and_hms(2026, 4, 5, 0, 1, 2)
            .single()
            .expect("timestamp should build");

        assert_eq!(
            format_local_rfc3339(timestamp),
            timestamp
                .with_timezone(&Local)
                .to_rfc3339_opts(SecondsFormat::Nanos, false)
        );
    }

    #[test]
    fn local_rfc3339_option_round_trips_back_to_utc() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct Wrapper {
            #[serde(with = "crate::local_rfc3339_option")]
            value: Option<DateTime<Utc>>,
        }

        let wrapper = Wrapper {
            value: Some(
                Utc.with_ymd_and_hms(2026, 4, 5, 12, 34, 56)
                    .single()
                    .expect("timestamp should build"),
            ),
        };

        let encoded = serde_json::to_string(&wrapper).expect("wrapper should serialize");
        let decoded: Wrapper = serde_json::from_str(&encoded).expect("wrapper should deserialize");

        assert_eq!(decoded, wrapper);
    }
}
