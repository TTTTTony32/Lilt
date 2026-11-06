use std::collections::HashMap;

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone, Timelike};

use crate::{
    AppState,
    contracts::{
        ProviderTokenUsageBucket, ProviderTokenUsageBucketKind, ProviderTokenUsageResponse,
        ProviderTokenUsageStats,
    },
    db,
    pdf_protocol::TokenUsage,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderTokenUsageSample {
    pub model_id: String,
    pub recorded_at_ms: i64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
}

pub(crate) fn record_provider_token_usage(
    state: &AppState,
    provider_id: &str,
    model_id: &str,
    request_id: &str,
    usage: Option<&TokenUsage>,
) {
    let result = state
        .database
        .lock()
        .map_err(|_| "应用数据库锁已损坏".to_string())
        .and_then(|connection| {
            db::insert_provider_token_usage(
                &connection,
                &db::ProviderTokenUsageRecord {
                    provider_id,
                    model_id,
                    request_id,
                    recorded_at_ms: chrono::Utc::now().timestamp_millis(),
                    input_tokens: usage.and_then(|usage| usage.prompt_tokens),
                    output_tokens: usage.and_then(|usage| usage.completion_tokens),
                    cache_read_tokens: usage.and_then(|usage| usage.cache_read_tokens),
                },
            )
        });
    if let Err(error) = result {
        crate::diagnostics::warn(format!(
            "provider.token_usage.persist_failed provider_id={} request_id={} reason={error}",
            provider_id, request_id
        ));
    }
}

pub(crate) fn aggregate_provider_token_usage(
    provider_id: &str,
    model_id: Option<&str>,
    from_ms: i64,
    to_ms: i64,
    bucket: ProviderTokenUsageBucketKind,
    models: Vec<String>,
    samples: &[ProviderTokenUsageSample],
) -> Result<ProviderTokenUsageResponse, String> {
    if from_ms >= to_ms {
        return Err("Token 用量查询范围无效".to_string());
    }

    let windows = bucket_windows(from_ms, to_ms, bucket)?;
    let mut bucket_totals = windows
        .iter()
        .map(|(start_ms, _)| (*start_ms, UsageAccumulator::default()))
        .collect::<HashMap<_, _>>();
    let mut total = UsageAccumulator::default();

    for sample in samples.iter().filter(|sample| {
        model_id.is_none_or(|selected| sample.model_id == selected)
            && sample.recorded_at_ms >= from_ms
            && sample.recorded_at_ms < to_ms
    }) {
        let start_ms = bucket_start(sample.recorded_at_ms, bucket)?;
        let accumulator = bucket_totals
            .get_mut(&start_ms)
            .ok_or_else(|| "Token 用量记录超出聚合范围".to_string())?;
        accumulator.add(sample);
        total.add(sample);
    }

    let buckets = windows
        .into_iter()
        .map(|(start_ms, end_ms)| ProviderTokenUsageBucket {
            start_ms,
            end_ms,
            stats: bucket_totals
                .remove(&start_ms)
                .unwrap_or_default()
                .into_stats(),
        })
        .collect();

    Ok(ProviderTokenUsageResponse {
        provider_id: provider_id.to_string(),
        from_ms,
        to_ms,
        bucket,
        model_id: model_id.map(str::to_string),
        models,
        buckets,
        totals: total.into_stats(),
    })
}

#[derive(Default)]
struct UsageAccumulator {
    request_count: u64,
    input_tokens: u64,
    input_known_count: u64,
    input_unknown_count: u64,
    output_tokens: u64,
    output_known_count: u64,
    output_unknown_count: u64,
    cache_read_tokens: u64,
    cache_known_count: u64,
    cache_known_read_tokens: u64,
    cache_known_input_tokens: u64,
    cache_paired_request_count: u64,
    cache_detail_unknown_count: u64,
}

impl UsageAccumulator {
    fn add(&mut self, sample: &ProviderTokenUsageSample) {
        self.request_count = self.request_count.saturating_add(1);
        if let Some(tokens) = sample.input_tokens {
            self.input_tokens = self.input_tokens.saturating_add(tokens);
            self.input_known_count = self.input_known_count.saturating_add(1);
        } else {
            self.input_unknown_count = self.input_unknown_count.saturating_add(1);
        }
        if let Some(tokens) = sample.output_tokens {
            self.output_tokens = self.output_tokens.saturating_add(tokens);
            self.output_known_count = self.output_known_count.saturating_add(1);
        } else {
            self.output_unknown_count = self.output_unknown_count.saturating_add(1);
        }
        if let Some(tokens) = sample.cache_read_tokens {
            self.cache_read_tokens = self.cache_read_tokens.saturating_add(tokens);
            self.cache_known_count = self.cache_known_count.saturating_add(1);
            if let Some(input_tokens) = sample.input_tokens {
                self.cache_known_read_tokens = self.cache_known_read_tokens.saturating_add(tokens);
                self.cache_known_input_tokens =
                    self.cache_known_input_tokens.saturating_add(input_tokens);
                self.cache_paired_request_count = self.cache_paired_request_count.saturating_add(1);
            }
        } else {
            self.cache_detail_unknown_count = self.cache_detail_unknown_count.saturating_add(1);
        }
    }

    fn into_stats(self) -> ProviderTokenUsageStats {
        ProviderTokenUsageStats {
            input_tokens: known_sum_or_empty(
                self.input_tokens,
                self.input_known_count,
                self.request_count,
            ),
            output_tokens: known_sum_or_empty(
                self.output_tokens,
                self.output_known_count,
                self.request_count,
            ),
            cache_read_tokens: (self.cache_known_count > 0).then_some(self.cache_read_tokens),
            cache_known_read_tokens: self.cache_known_read_tokens,
            cache_known_input_tokens: self.cache_known_input_tokens,
            cache_paired_request_count: self.cache_paired_request_count,
            input_unknown_count: self.input_unknown_count,
            output_unknown_count: self.output_unknown_count,
            cache_detail_unknown_count: self.cache_detail_unknown_count,
            request_count: self.request_count,
        }
    }
}

fn known_sum_or_empty(sum: u64, known_count: u64, request_count: u64) -> Option<u64> {
    if known_count > 0 || request_count == 0 {
        Some(sum)
    } else {
        None
    }
}

fn bucket_windows(
    from_ms: i64,
    to_ms: i64,
    bucket: ProviderTokenUsageBucketKind,
) -> Result<Vec<(i64, i64)>, String> {
    let mut windows = Vec::new();
    let mut start_ms = bucket_start(from_ms, bucket)?;
    while start_ms < to_ms {
        let end_ms = next_bucket_start(start_ms, bucket)?;
        if end_ms <= start_ms {
            return Err("Token 用量时间桶边界无效".to_string());
        }
        if end_ms > from_ms {
            windows.push((start_ms, end_ms));
            if windows.len() > 2_000 {
                return Err("Token 用量查询范围过大".to_string());
            }
        }
        start_ms = end_ms;
    }
    Ok(windows)
}

fn bucket_start(timestamp_ms: i64, bucket: ProviderTokenUsageBucketKind) -> Result<i64, String> {
    let time = Local
        .timestamp_millis_opt(timestamp_ms)
        .single()
        .ok_or_else(|| "Token 用量时间戳无效".to_string())?;
    let start = match bucket {
        ProviderTokenUsageBucketKind::Hour => {
            time - Duration::minutes(time.minute() as i64)
                - Duration::seconds(time.second() as i64)
                - Duration::nanoseconds(time.nanosecond() as i64)
        }
        ProviderTokenUsageBucketKind::Day => local_midnight(time.date_naive())?,
        ProviderTokenUsageBucketKind::Week => {
            let date =
                time.date_naive() - Duration::days(time.weekday().num_days_from_monday() as i64);
            local_midnight(date)?
        }
        ProviderTokenUsageBucketKind::Month => {
            let date = NaiveDate::from_ymd_opt(time.year(), time.month(), 1)
                .ok_or_else(|| "Token 用量月份无效".to_string())?;
            local_midnight(date)?
        }
    };
    Ok(start.timestamp_millis())
}

fn next_bucket_start(start_ms: i64, bucket: ProviderTokenUsageBucketKind) -> Result<i64, String> {
    if bucket == ProviderTokenUsageBucketKind::Hour {
        return start_ms
            .checked_add(Duration::hours(1).num_milliseconds())
            .ok_or_else(|| "Token 用量时间范围溢出".to_string());
    }
    let start = Local
        .timestamp_millis_opt(start_ms)
        .single()
        .ok_or_else(|| "Token 用量时间戳无效".to_string())?;
    let date = start.date_naive();
    let next_date = match bucket {
        ProviderTokenUsageBucketKind::Hour => unreachable!(),
        ProviderTokenUsageBucketKind::Day => date.succ_opt(),
        ProviderTokenUsageBucketKind::Week => date.checked_add_signed(Duration::days(7)),
        ProviderTokenUsageBucketKind::Month => {
            let next_year = if date.month() == 12 {
                date.year().checked_add(1)
            } else {
                Some(date.year())
            }
            .ok_or_else(|| "Token 用量月份溢出".to_string())?;
            let next_month = if date.month() == 12 {
                1
            } else {
                date.month() + 1
            };
            NaiveDate::from_ymd_opt(next_year, next_month, 1)
        }
    }
    .ok_or_else(|| "Token 用量时间范围溢出".to_string())?;
    Ok(local_midnight(next_date)?.timestamp_millis())
}

fn local_midnight(date: NaiveDate) -> Result<chrono::DateTime<Local>, String> {
    let midnight = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| "Token 用量日期无效".to_string())?;
    for minute in 0..=24 * 60 {
        let candidate = midnight
            .checked_add_signed(Duration::minutes(minute))
            .ok_or_else(|| "Token 用量日期溢出".to_string())?;
        match Local.from_local_datetime(&candidate) {
            chrono::LocalResult::Single(value) => return Ok(value),
            chrono::LocalResult::Ambiguous(first, second) => {
                return Ok(if first.timestamp_millis() <= second.timestamp_millis() {
                    first
                } else {
                    second
                });
            }
            chrono::LocalResult::None => {}
        }
    }
    Err("Token 用量日期没有本地时间边界".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(
        model_id: &str,
        recorded_at_ms: i64,
        input: Option<u64>,
        output: Option<u64>,
        cache: Option<u64>,
    ) -> ProviderTokenUsageSample {
        ProviderTokenUsageSample {
            model_id: model_id.to_string(),
            recorded_at_ms,
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache,
        }
    }

    #[test]
    fn aggregate_keeps_unknown_cache_usage_and_filters_model() {
        let from_ms = local_midnight(NaiveDate::from_ymd_opt(2027, 1, 1).unwrap())
            .unwrap()
            .timestamp_millis();
        let to_ms = local_midnight(NaiveDate::from_ymd_opt(2027, 1, 2).unwrap())
            .unwrap()
            .timestamp_millis();
        let response = aggregate_provider_token_usage(
            "provider-a",
            Some("model-a"),
            from_ms,
            to_ms,
            ProviderTokenUsageBucketKind::Day,
            vec!["model-a".to_string(), "model-old".to_string()],
            &[
                sample("model-a", from_ms + 1, Some(100), Some(20), Some(25)),
                sample("model-a", from_ms + 2, Some(50), Some(10), None),
                sample("model-a", from_ms + 3, None, Some(5), Some(8)),
                sample("model-b", from_ms + 3, Some(900), Some(90), Some(12)),
                sample("model-a", from_ms - 1, Some(700), Some(70), Some(7)),
            ],
        )
        .unwrap();

        assert_eq!(response.provider_id, "provider-a");
        assert_eq!(response.model_id.as_deref(), Some("model-a"));
        assert_eq!(response.models, ["model-a", "model-old"]);
        assert_eq!(response.totals.request_count, 3);
        assert_eq!(response.totals.input_tokens, Some(150));
        assert_eq!(response.totals.input_unknown_count, 1);
        assert_eq!(response.totals.cache_read_tokens, Some(33));
        assert_eq!(response.totals.cache_known_read_tokens, 25);
        assert_eq!(response.totals.cache_known_input_tokens, 100);
        assert_eq!(response.totals.cache_paired_request_count, 1);
        assert_eq!(response.totals.cache_detail_unknown_count, 1);
        assert_eq!(response.buckets.len(), 1);
        assert_eq!(response.buckets[0].stats.request_count, 3);
        assert_eq!(response.buckets[0].stats.cache_paired_request_count, 1);
    }

    #[test]
    fn no_calls_are_zero_but_missing_usage_is_unknown() {
        let from_ms = 1_800_000_000_000_i64;
        let to_ms = from_ms + 1;
        let empty = aggregate_provider_token_usage(
            "provider-a",
            None,
            from_ms,
            to_ms,
            ProviderTokenUsageBucketKind::Hour,
            Vec::new(),
            &[],
        )
        .unwrap();
        assert_eq!(empty.totals.input_tokens, Some(0));
        assert_eq!(empty.totals.cache_read_tokens, None);

        let unknown = aggregate_provider_token_usage(
            "provider-a",
            None,
            from_ms,
            to_ms,
            ProviderTokenUsageBucketKind::Hour,
            Vec::new(),
            &[sample("model-a", from_ms, None, None, None)],
        )
        .unwrap();
        assert_eq!(unknown.totals.input_tokens, None);
        assert_eq!(unknown.totals.output_tokens, None);
        assert_eq!(unknown.totals.input_unknown_count, 1);
        assert_eq!(unknown.totals.cache_detail_unknown_count, 1);
        assert_eq!(unknown.totals.cache_paired_request_count, 0);

        let paired_zero = aggregate_provider_token_usage(
            "provider-a",
            None,
            from_ms,
            to_ms,
            ProviderTokenUsageBucketKind::Hour,
            Vec::new(),
            &[sample("model-a", from_ms, Some(0), Some(0), Some(0))],
        )
        .unwrap();
        assert_eq!(paired_zero.totals.cache_known_input_tokens, 0);
        assert_eq!(paired_zero.totals.cache_known_read_tokens, 0);
        assert_eq!(paired_zero.totals.cache_paired_request_count, 1);
    }
}
