use super::bad;
use crate::error::Result;
use chrono::{Datelike, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;

pub const PERIODS: &[&str] = &["day", "week", "month", "quarter", "year"];
pub fn timezone(value: &str) -> Result<Tz> {
    value.parse().map_err(|_| bad("timezone"))
}
pub fn floor(date: NaiveDate, period: &str) -> Result<NaiveDate> {
    Ok(match period {
        "day" => date,
        "week" => date - Duration::days(date.weekday().num_days_from_monday() as i64),
        "month" => date.with_day(1).unwrap(),
        "quarter" => NaiveDate::from_ymd_opt(date.year(), (date.month0() / 3) * 3 + 1, 1).unwrap(),
        "year" => NaiveDate::from_ymd_opt(date.year(), 1, 1).unwrap(),
        _ => return Err(bad("period")),
    })
}
pub fn advance(date: NaiveDate, period: &str) -> Result<NaiveDate> {
    let months = match period {
        "day" => {
            return date
                .checked_add_signed(Duration::days(1))
                .ok_or_else(|| bad("date"));
        }
        "week" => {
            return date
                .checked_add_signed(Duration::days(7))
                .ok_or_else(|| bad("date"));
        }
        "month" => 1,
        "quarter" => 3,
        "year" => 12,
        _ => return Err(bad("period")),
    };
    date.checked_add_months(chrono::Months::new(months))
        .ok_or_else(|| bad("date"))
}
/// Resolve a local calendar boundary, including zones that shift at midnight.
pub fn boundary(date: NaiveDate, tz: Tz) -> Result<i64> {
    let midnight = date.and_hms_opt(0, 0, 0).unwrap();
    for minute in 0..=1440 {
        let local = midnight + Duration::minutes(minute);
        if let Some(value) = tz.from_local_datetime(&local).earliest() {
            return Ok(value.timestamp_millis());
        }
    }
    Err(bad("boundary"))
}
pub fn bounds(period: &str, date: NaiveDate, tz: Tz) -> Result<(i64, i64)> {
    let start = floor(date, period)?;
    Ok((boundary(start, tz)?, boundary(advance(start, period)?, tz)?))
}
pub fn local_date(now: i64, tz: Tz) -> Result<NaiveDate> {
    Utc.timestamp_millis_opt(now)
        .single()
        .map(|d| d.with_timezone(&tz).date_naive())
        .ok_or_else(|| bad("date"))
}
pub fn next(periods: &[String], tz: Tz, after: i64) -> Result<i64> {
    periods
        .iter()
        .map(|period| {
            let date = floor(local_date(after - 1_800_000, tz)?, period)?;
            Ok(boundary(advance(date, period)?, tz)? + 1_800_000)
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .min()
        .ok_or_else(|| bad("periods"))
}
