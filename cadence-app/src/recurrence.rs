use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;

const MAX_DAYS: i64 = 366 * 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frequency {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Day {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Day {
    fn as_chrono(self) -> Weekday {
        match self {
            Self::Monday => Weekday::Mon,
            Self::Tuesday => Weekday::Tue,
            Self::Wednesday => Weekday::Wed,
            Self::Thursday => Weekday::Thu,
            Self::Friday => Weekday::Fri,
            Self::Saturday => Weekday::Sat,
            Self::Sunday => Weekday::Sun,
        }
    }

    fn parse(value: &str) -> Result<Self, RecurrenceError> {
        match value {
            "MO" => Ok(Self::Monday),
            "TU" => Ok(Self::Tuesday),
            "WE" => Ok(Self::Wednesday),
            "TH" => Ok(Self::Thursday),
            "FR" => Ok(Self::Friday),
            "SA" => Ok(Self::Saturday),
            "SU" => Ok(Self::Sunday),
            _ => Err(RecurrenceError::InvalidValue("BYDAY", value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub frequency: Frequency,
    pub interval: u32,
    pub by_day: Vec<Day>,
    pub by_month_day: Vec<u32>,
    pub count: Option<u32>,
    pub until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct Recurrence {
    pub rule: Rule,
    pub starts_at: DateTime<Utc>,
    pub timezone: Tz,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RecurrenceError {
    #[error("missing FREQ")]
    MissingFrequency,
    #[error("invalid recurrence value for {0}: {1}")]
    InvalidValue(&'static str, String),
    #[error("invalid timezone: {0}")]
    InvalidTimezone(String),
    #[error("rule produces no occurrences")]
    Empty,
}

impl Recurrence {
    pub fn parse(
        rrule: &str,
        timezone: &str,
        starts_at: DateTime<Utc>,
    ) -> Result<Self, RecurrenceError> {
        let timezone = timezone
            .parse::<Tz>()
            .map_err(|_| RecurrenceError::InvalidTimezone(timezone.to_owned()))?;
        let rule = parse_rule(rrule)?;

        Ok(Self {
            rule,
            starts_at,
            timezone,
        })
    }

    pub fn next_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let local_start = self.starts_at.with_timezone(&self.timezone);
        let start_date = local_start.date_naive();
        let start_time = local_start.time();
        let mut matches = 0_u32;

        for offset in 0..MAX_DAYS {
            let date = start_date + Duration::days(offset);
            if !self.matches_date(start_date, date) {
                continue;
            }

            let naive = NaiveDateTime::new(date, start_time);
            let Some(candidate) = self.timezone.from_local_datetime(&naive).earliest() else {
                continue;
            };
            let candidate = candidate.with_timezone(&Utc);

            if let Some(until) = self.rule.until
                && candidate > until
            {
                return None;
            }

            matches += 1;
            if let Some(count) = self.rule.count
                && matches > count
            {
                return None;
            }

            if candidate > after {
                return Some(candidate);
            }
        }

        None
    }

    fn matches_date(&self, start: NaiveDate, date: NaiveDate) -> bool {
        match self.rule.frequency {
            Frequency::Daily => {
                let offset = (date - start).num_days();
                offset >= 0 && offset % i64::from(self.rule.interval) == 0
            }
            Frequency::Weekly => {
                let offset = weeks_between(start, date);
                if offset < 0 || offset % i64::from(self.rule.interval) != 0 {
                    return false;
                }

                if self.rule.by_day.is_empty() {
                    date.weekday() == start.weekday()
                } else {
                    self.rule
                        .by_day
                        .iter()
                        .any(|day| day.as_chrono() == date.weekday())
                }
            }
            Frequency::Monthly => {
                let offset = months_between(start, date);
                if offset < 0 || offset % i64::from(self.rule.interval) != 0 {
                    return false;
                }

                let day_matches = if self.rule.by_month_day.is_empty() {
                    date.day() == start.day()
                } else {
                    self.rule.by_month_day.contains(&date.day())
                };
                let weekday_matches = self.rule.by_day.is_empty()
                    || self
                        .rule
                        .by_day
                        .iter()
                        .any(|day| day.as_chrono() == date.weekday());

                day_matches && weekday_matches
            }
            Frequency::Yearly => {
                let year_offset = date.year() - start.year();
                if year_offset < 0 || year_offset % self.rule.interval as i32 != 0 {
                    return false;
                }

                let day_matches = if self.rule.by_month_day.is_empty() {
                    date.day() == start.day()
                } else {
                    self.rule.by_month_day.contains(&date.day())
                };

                date.month() == start.month() && day_matches
            }
        }
    }
}

fn parse_rule(rrule: &str) -> Result<Rule, RecurrenceError> {
    let mut frequency = None;
    let mut interval = 1_u32;
    let mut by_day = Vec::new();
    let mut by_month_day = Vec::new();
    let mut count = None;
    let mut until = None;

    for part in rrule.split(';').filter(|part| !part.is_empty()) {
        let Some((key, value)) = part.split_once('=') else {
            return Err(RecurrenceError::InvalidValue("rule", part.to_owned()));
        };

        match key {
            "FREQ" => {
                frequency = Some(match value {
                    "DAILY" => Frequency::Daily,
                    "WEEKLY" => Frequency::Weekly,
                    "MONTHLY" => Frequency::Monthly,
                    "YEARLY" => Frequency::Yearly,
                    _ => return Err(RecurrenceError::InvalidValue("FREQ", value.to_owned())),
                });
            }
            "INTERVAL" => {
                interval = value
                    .parse()
                    .map_err(|_| RecurrenceError::InvalidValue("INTERVAL", value.to_owned()))?;
                if interval == 0 {
                    return Err(RecurrenceError::InvalidValue("INTERVAL", value.to_owned()));
                }
            }
            "BYDAY" => {
                by_day = value
                    .split(',')
                    .map(Day::parse)
                    .collect::<Result<Vec<_>, _>>()?;
            }
            "BYMONTHDAY" => {
                by_month_day = value
                    .split(',')
                    .map(|day| {
                        day.parse::<u32>()
                            .ok()
                            .filter(|day| (1..=31).contains(day))
                            .ok_or_else(|| {
                                RecurrenceError::InvalidValue("BYMONTHDAY", day.to_owned())
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
            }
            "COUNT" => {
                let parsed = value
                    .parse::<u32>()
                    .map_err(|_| RecurrenceError::InvalidValue("COUNT", value.to_owned()))?;
                if parsed == 0 {
                    return Err(RecurrenceError::InvalidValue("COUNT", value.to_owned()));
                }
                count = Some(parsed);
            }
            "UNTIL" => {
                until = Some(parse_until(value)?);
            }
            "WKST" => {}
            _ => return Err(RecurrenceError::InvalidValue("key", key.to_owned())),
        }
    }

    Ok(Rule {
        frequency: frequency.ok_or(RecurrenceError::MissingFrequency)?,
        interval,
        by_day,
        by_month_day,
        count,
        until,
    })
}

fn parse_until(value: &str) -> Result<DateTime<Utc>, RecurrenceError> {
    if let Ok(value) = DateTime::parse_from_rfc3339(value) {
        return Ok(value.with_timezone(&Utc));
    }

    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| RecurrenceError::InvalidValue("UNTIL", value.to_owned()))?;
    Ok(date
        .and_hms_opt(23, 59, 59)
        .expect("valid end of day")
        .and_utc())
}

fn weeks_between(start: NaiveDate, date: NaiveDate) -> i64 {
    let start = start - Duration::days(i64::from(start.weekday().num_days_from_monday()));
    let date = date - Duration::days(i64::from(date.weekday().num_days_from_monday()));
    (date - start).num_weeks()
}

fn months_between(start: NaiveDate, date: NaiveDate) -> i64 {
    i64::from(date.year() - start.year()) * 12 + i64::from(date.month()) - i64::from(start.month())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn advances_daily_with_an_interval() {
        let recurrence =
            Recurrence::parse("FREQ=DAILY;INTERVAL=2", "UTC", utc("2026-01-01T09:00:00Z")).unwrap();

        assert_eq!(
            recurrence.next_after(utc("2026-01-01T10:00:00Z")),
            Some(utc("2026-01-03T09:00:00Z"))
        );
    }

    #[test]
    fn advances_weekly_on_selected_days() {
        let recurrence = Recurrence::parse(
            "FREQ=WEEKLY;BYDAY=MO,WE",
            "UTC",
            utc("2026-01-05T09:00:00Z"),
        )
        .unwrap();

        assert_eq!(
            recurrence.next_after(utc("2026-01-05T10:00:00Z")),
            Some(utc("2026-01-07T09:00:00Z"))
        );
        assert_eq!(
            recurrence.next_after(utc("2026-01-07T10:00:00Z")),
            Some(utc("2026-01-12T09:00:00Z"))
        );
    }

    #[test]
    fn skips_invalid_month_days() {
        let recurrence = Recurrence::parse(
            "FREQ=MONTHLY;BYMONTHDAY=31",
            "UTC",
            utc("2026-01-31T09:00:00Z"),
        )
        .unwrap();

        assert_eq!(
            recurrence.next_after(utc("2026-02-01T00:00:00Z")),
            Some(utc("2026-03-31T09:00:00Z"))
        );
    }

    #[test]
    fn respects_count_and_until() {
        let count =
            Recurrence::parse("FREQ=DAILY;COUNT=3", "UTC", utc("2026-01-01T09:00:00Z")).unwrap();
        let third = utc("2026-01-03T09:00:00Z");
        assert_eq!(count.next_after(utc("2026-01-02T10:00:00Z")), Some(third));
        assert_eq!(count.next_after(third), None);

        let until = Recurrence::parse(
            "FREQ=DAILY;UNTIL=2026-01-02T09:00:00Z",
            "UTC",
            utc("2026-01-01T09:00:00Z"),
        )
        .unwrap();
        assert_eq!(
            until.next_after(utc("2026-01-01T10:00:00Z")),
            Some(utc("2026-01-02T09:00:00Z"))
        );
        assert_eq!(until.next_after(utc("2026-01-02T09:00:00Z")), None);
    }

    #[test]
    fn keeps_local_wall_time_across_dst() {
        let recurrence = Recurrence::parse(
            "FREQ=DAILY",
            "America/New_York",
            utc("2026-03-07T14:00:00Z"),
        )
        .unwrap();

        let next = recurrence.next_after(utc("2026-03-07T15:00:00Z")).unwrap();
        assert_eq!(next, utc("2026-03-08T13:00:00Z"));
    }

    #[test]
    fn rejects_invalid_rules() {
        assert!(matches!(
            Recurrence::parse("INTERVAL=2", "UTC", Utc::now()),
            Err(RecurrenceError::MissingFrequency)
        ));
        assert!(matches!(
            Recurrence::parse("FREQ=DAILY;INTERVAL=0", "UTC", Utc::now()),
            Err(RecurrenceError::InvalidValue("INTERVAL", _))
        ));
        assert!(matches!(
            Recurrence::parse("FREQ=HOURLY", "UTC", Utc::now()),
            Err(RecurrenceError::InvalidValue("FREQ", _))
        ));
    }
}
