//! Pure aggregation over feedings; no I/O so it is cheap to test.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use serde::Serialize;

use crate::feedings::{BreastSide, Feeding, FeedingKind};

/// Bottles contribute millilitres, nursing contributes minutes; `feeding_count` counts both.
#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct FeedingTotals {
    pub feeding_count: i64,
    pub bottle_count: i64,
    pub total_ml: i64,
    pub breast_milk_ml: i64,
    pub formula_ml: i64,
    pub nursing_count: i64,
    pub nursing_minutes: i64,
    pub nursing_left_minutes: i64,
    pub nursing_right_minutes: i64,
    pub nursing_both_minutes: i64,
}

impl FeedingTotals {
    fn add(&mut self, feeding: &Feeding) {
        self.feeding_count += 1;

        let details = &feeding.details;

        match details.kind {
            FeedingKind::Nursing => self.add_nursing(
                details.duration_minutes.unwrap_or_default(),
                details.breast_side,
            ),
            FeedingKind::BreastMilk => {
                let amount_ml = self.add_bottle(details.amount_ml.unwrap_or_default());
                self.breast_milk_ml += amount_ml;
            }
            FeedingKind::Formula => {
                let amount_ml = self.add_bottle(details.amount_ml.unwrap_or_default());
                self.formula_ml += amount_ml;
            }
        }
    }

    fn add_bottle(&mut self, amount_ml: i64) -> i64 {
        self.bottle_count += 1;
        self.total_ml += amount_ml;

        amount_ml
    }

    fn add_nursing(&mut self, duration_minutes: i64, breast_side: Option<BreastSide>) {
        self.nursing_count += 1;
        self.nursing_minutes += duration_minutes;

        match breast_side {
            Some(BreastSide::Left) => self.nursing_left_minutes += duration_minutes,
            Some(BreastSide::Right) => self.nursing_right_minutes += duration_minutes,
            Some(BreastSide::Both) => self.nursing_both_minutes += duration_minutes,
            None => {}
        }
    }

    fn average_amount_ml(&self) -> Option<f64> {
        average(self.total_ml, self.bottle_count)
    }

    fn average_nursing_minutes(&self) -> Option<f64> {
        average(self.nursing_minutes, self.nursing_count)
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub struct DailyStatistics {
    pub date: NaiveDate,
    #[serde(flatten)]
    pub totals: FeedingTotals,
    /// Per bottle.
    pub average_amount_ml: Option<f64>,
    /// Per nursing session.
    pub average_nursing_minutes: Option<f64>,
    pub average_interval_minutes: Option<f64>,
    pub longest_interval_minutes: Option<i64>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct PeriodStatistics {
    pub timezone: String,
    pub days: Vec<DailyStatistics>,
    #[serde(flatten)]
    pub totals: FeedingTotals,
    pub average_daily_ml: f64,
    pub average_daily_nursing_minutes: f64,
    pub average_amount_ml: Option<f64>,
    pub average_nursing_minutes: Option<f64>,
}

pub fn sum_feedings(feedings: &[Feeding]) -> FeedingTotals {
    let mut totals = FeedingTotals::default();

    for feeding in feedings {
        totals.add(feeding);
    }

    totals
}

/// The first instant of `date` in `timezone`, tolerating DST transitions that skip local midnight.
pub fn start_of_local_day(date: NaiveDate, timezone: Tz) -> DateTime<Utc> {
    let local_midnight = date.and_hms_opt(0, 0, 0).expect("midnight is a valid time");

    timezone
        .from_local_datetime(&local_midnight)
        .earliest()
        .map(|local_start| local_start.with_timezone(&Utc))
        .unwrap_or_else(|| Utc.from_utc_datetime(&local_midnight))
}

#[derive(Default)]
struct DayAccumulator {
    totals: FeedingTotals,
    interval_minutes: Vec<i64>,
}

/// Buckets oldest-first `feedings` into local calendar days from `first_day` to `last_day` inclusive.
/// An interval is the gap since the previous feeding and is credited to the day of the later one.
pub fn build_period_statistics(
    feedings: &[Feeding],
    timezone: Tz,
    first_day: NaiveDate,
    last_day: NaiveDate,
) -> PeriodStatistics {
    let mut days_by_date: BTreeMap<NaiveDate, DayAccumulator> = first_day
        .iter_days()
        .take_while(|date| *date <= last_day)
        .map(|date| (date, DayAccumulator::default()))
        .collect();

    let mut period_totals = FeedingTotals::default();
    let mut previous_fed_at: Option<DateTime<Utc>> = None;

    for feeding in feedings {
        let local_date = feeding.fed_at.with_timezone(&timezone).date_naive();

        let Some(day) = days_by_date.get_mut(&local_date) else {
            continue;
        };

        day.totals.add(feeding);
        period_totals.add(feeding);

        if let Some(previous) = previous_fed_at {
            day.interval_minutes
                .push((feeding.fed_at - previous).num_minutes());
        }

        previous_fed_at = Some(feeding.fed_at);
    }

    let days: Vec<DailyStatistics> = days_by_date
        .into_iter()
        .map(|(date, accumulator)| summarize_day(date, accumulator))
        .collect();

    let day_count = days.len().max(1) as f64;

    PeriodStatistics {
        timezone: timezone.name().to_owned(),
        average_daily_ml: round_to_one_decimal(period_totals.total_ml as f64 / day_count),
        average_daily_nursing_minutes: round_to_one_decimal(
            period_totals.nursing_minutes as f64 / day_count,
        ),
        average_amount_ml: period_totals.average_amount_ml(),
        average_nursing_minutes: period_totals.average_nursing_minutes(),
        totals: period_totals,
        days,
    }
}

fn summarize_day(date: NaiveDate, accumulator: DayAccumulator) -> DailyStatistics {
    let intervals = &accumulator.interval_minutes;

    DailyStatistics {
        date,
        average_amount_ml: accumulator.totals.average_amount_ml(),
        average_nursing_minutes: accumulator.totals.average_nursing_minutes(),
        totals: accumulator.totals,
        average_interval_minutes: average(intervals.iter().sum(), intervals.len() as i64),
        longest_interval_minutes: intervals.iter().copied().max(),
    }
}

fn average(sum: i64, count: i64) -> Option<f64> {
    if count == 0 {
        return None;
    }

    Some(round_to_one_decimal(sum as f64 / count as f64))
}

fn round_to_one_decimal(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feedings::FeedingDetails;
    use uuid::Uuid;

    fn feeding_with(fed_at: &str, details: FeedingDetails) -> Feeding {
        let fed_at: DateTime<Utc> = fed_at.parse().unwrap();

        Feeding {
            id: Uuid::now_v7(),
            fed_at,
            details,
            created_by: None,
            created_at: fed_at,
            updated_at: fed_at,
        }
    }

    fn bottle_at(fed_at: &str, amount_ml: i64, kind: FeedingKind) -> Feeding {
        feeding_with(
            fed_at,
            FeedingDetails {
                kind,
                amount_ml: Some(amount_ml),
                duration_minutes: None,
                breast_side: None,
            },
        )
    }

    fn nursing_at(fed_at: &str, duration_minutes: i64, breast_side: BreastSide) -> Feeding {
        feeding_with(
            fed_at,
            FeedingDetails {
                kind: FeedingKind::Nursing,
                amount_ml: None,
                duration_minutes: Some(duration_minutes),
                breast_side: Some(breast_side),
            },
        )
    }

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn buckets_by_local_day_not_utc_day() {
        // 23:30 UTC on Oct 7 is 01:30 on Oct 8 in Prague (UTC+2 in summer time).
        let feedings = vec![
            bottle_at("2026-10-07T10:00:00Z", 80, FeedingKind::Formula),
            bottle_at("2026-10-07T23:30:00Z", 60, FeedingKind::BreastMilk),
        ];

        let statistics = build_period_statistics(
            &feedings,
            chrono_tz::Europe::Prague,
            date("2026-10-07"),
            date("2026-10-08"),
        );

        assert_eq!(statistics.days[0].totals.total_ml, 80);
        assert_eq!(statistics.days[1].totals.total_ml, 60);
        assert_eq!(statistics.days[1].totals.breast_milk_ml, 60);
        assert_eq!(statistics.totals.total_ml, 140);
        assert_eq!(statistics.average_daily_ml, 70.0);
    }

    #[test]
    fn nursing_adds_minutes_per_side_but_no_millilitres() {
        let feedings = vec![
            nursing_at("2026-10-08T01:00:00Z", 15, BreastSide::Left),
            nursing_at("2026-10-08T04:00:00Z", 20, BreastSide::Right),
            nursing_at("2026-10-08T07:00:00Z", 10, BreastSide::Both),
            bottle_at("2026-10-08T10:00:00Z", 90, FeedingKind::Formula),
        ];

        let totals = sum_feedings(&feedings);

        assert_eq!(totals.feeding_count, 4);
        assert_eq!(totals.nursing_count, 3);
        assert_eq!(totals.nursing_minutes, 45);
        assert_eq!(totals.nursing_left_minutes, 15);
        assert_eq!(totals.nursing_right_minutes, 20);
        assert_eq!(totals.nursing_both_minutes, 10);
        assert_eq!(totals.bottle_count, 1);
        assert_eq!(totals.total_ml, 90);
        assert_eq!(totals.average_amount_ml(), Some(90.0));
        assert_eq!(totals.average_nursing_minutes(), Some(15.0));
    }

    #[test]
    fn computes_intervals_across_kinds_and_midnight() {
        let feedings = vec![
            nursing_at("2026-10-07T21:00:00Z", 15, BreastSide::Left),
            bottle_at("2026-10-08T00:00:00Z", 90, FeedingKind::Formula),
            nursing_at("2026-10-08T02:00:00Z", 15, BreastSide::Right),
        ];

        let statistics = build_period_statistics(
            &feedings,
            chrono_tz::UTC,
            date("2026-10-07"),
            date("2026-10-08"),
        );

        let first_day = &statistics.days[0];
        let second_day = &statistics.days[1];

        assert_eq!(first_day.average_interval_minutes, None);
        assert_eq!(second_day.average_interval_minutes, Some(150.0));
        assert_eq!(second_day.longest_interval_minutes, Some(180));
    }

    #[test]
    fn includes_empty_days_so_charts_have_no_gaps() {
        let statistics =
            build_period_statistics(&[], chrono_tz::UTC, date("2026-10-01"), date("2026-10-07"));

        assert_eq!(statistics.days.len(), 7);
        assert!(
            statistics
                .days
                .iter()
                .all(|day| day.totals.feeding_count == 0)
        );
        assert_eq!(statistics.average_amount_ml, None);
        assert_eq!(statistics.average_nursing_minutes, None);
    }

    #[test]
    fn start_of_local_day_respects_offset() {
        let start = start_of_local_day(date("2026-10-08"), chrono_tz::Europe::Prague);

        assert_eq!(
            start,
            "2026-10-07T22:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }
}
