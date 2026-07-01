use std::{error::Error, fmt, future::Future, time::Duration};

use chrono::{
    DateTime, Datelike, Days, Duration as ChronoDuration, LocalResult, NaiveDate, NaiveTime,
    TimeZone, Utc,
};
use chrono_tz::Tz;
use sqlx::{pool::PoolConnection, PgPool, Postgres};
use tokio::time::sleep;
use tracing::{error, info, warn};

use crate::{
    config::SchedulerConfig,
    models::run::{Run, RunStatus},
    pipeline::{Pipeline, PipelineError},
};

const SCHEDULER_LOCK_CLASS: i32 = 0x4456;
const MAX_DST_GAP_MINUTES: i64 = 180;

#[derive(Clone)]
pub struct DailyScheduler {
    pool: PgPool,
    pipeline: Pipeline,
    schedule: DailySchedule,
}

impl DailyScheduler {
    pub fn new(pool: PgPool, pipeline: Pipeline, schedule: DailySchedule) -> Self {
        Self {
            pool,
            pipeline,
            schedule,
        }
    }

    pub fn from_config(
        pool: PgPool,
        pipeline: Pipeline,
        config: &SchedulerConfig,
    ) -> Result<Self, SchedulerError> {
        Ok(Self::new(pool, pipeline, DailySchedule::from_config(config)?))
    }

    pub async fn run_due_date(&self, date: NaiveDate) -> Result<DailyRunOutcome, SchedulerError> {
        let Some(date_lock) = self.try_acquire_date_lock(date).await? else {
            return Ok(DailyRunOutcome::SkippedAlreadyRunning { date });
        };

        let outcome = self.run_due_date_with_lock(date).await;
        let release = date_lock.release().await;

        match (outcome, release) {
            (Ok(outcome), Ok(())) => Ok(outcome),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(release_error)) => {
                warn!(
                    date = %date,
                    release_error = %release_error,
                    "failed to release scheduler advisory lock after pipeline error"
                );
                Err(error)
            }
        }
    }

    pub async fn run_until_shutdown<F>(&self, shutdown: F) -> Result<(), SchedulerError>
    where
        F: Future<Output = ()>,
    {
        tokio::pin!(shutdown);
        let mut last_attempted_date = None;

        loop {
            let now = Utc::now();
            if let Some(date) = self.schedule.due_date_at(now, last_attempted_date)? {
                last_attempted_date = Some(date);

                match self.run_due_date(date).await {
                    Ok(outcome) => {
                        info!(?outcome, "daily scheduler tick completed");
                    }
                    Err(error) => {
                        error!(date = %date, error = %error, "daily scheduler tick failed");
                    }
                }
            }

            let sleep_for = self.schedule.duration_until_next_fire(Utc::now())?;
            tokio::select! {
                () = sleep(sleep_for) => {},
                () = &mut shutdown => {
                    info!("daily scheduler stopped");
                    return Ok(());
                }
            }
        }
    }

    async fn run_due_date_with_lock(
        &self,
        date: NaiveDate,
    ) -> Result<DailyRunOutcome, SchedulerError> {
        match self.find_run_for_date(date).await? {
            Some(run) if run.status == RunStatus::Complete => {
                Ok(DailyRunOutcome::SkippedAlreadyComplete {
                    date,
                    run_id: run.id,
                })
            }
            Some(run) => {
                let outcome = self.pipeline.resume_run(run.id).await?;
                Ok(DailyRunOutcome::Resumed {
                    date,
                    run_id: outcome.run.id,
                })
            }
            None => {
                let outcome = self.pipeline.start_daily_run(date).await?;
                Ok(DailyRunOutcome::Started {
                    date,
                    run_id: outcome.run.id,
                })
            }
        }
    }

    async fn find_run_for_date(&self, date: NaiveDate) -> Result<Option<Run>, SchedulerError> {
        sqlx::query_as::<_, Run>(
            r#"
            SELECT id, date, animal, status, current_step, error, created_at, updated_at
            FROM runs
            WHERE date = $1
            "#,
        )
        .bind(date)
        .fetch_optional(&self.pool)
        .await
        .map_err(SchedulerError::Database)
    }

    async fn try_acquire_date_lock(
        &self,
        date: NaiveDate,
    ) -> Result<Option<DateLock>, SchedulerError> {
        let mut connection = self.pool.acquire().await.map_err(SchedulerError::Database)?;
        let lock_key = date.num_days_from_ce();
        let acquired = sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock($1, $2)")
            .bind(SCHEDULER_LOCK_CLASS)
            .bind(lock_key)
            .fetch_one(&mut *connection)
            .await
            .map_err(SchedulerError::Database)?;

        if acquired {
            Ok(Some(DateLock {
                connection,
                date,
                lock_key,
            }))
        } else {
            Ok(None)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DailySchedule {
    pub time: NaiveTime,
    pub timezone: Tz,
}

impl DailySchedule {
    pub fn from_config(config: &SchedulerConfig) -> Result<Self, SchedulerError> {
        let time = NaiveTime::parse_from_str(&config.time, "%H:%M").map_err(|_| {
            SchedulerError::InvalidScheduleTime {
                value: config.time.clone(),
            }
        })?;
        let timezone =
            config
                .timezone
                .parse::<Tz>()
                .map_err(|_| SchedulerError::InvalidTimezone {
                    value: config.timezone.clone(),
                })?;

        Ok(Self { time, timezone })
    }

    pub fn due_date_at(
        &self,
        now_utc: DateTime<Utc>,
        last_attempted_date: Option<NaiveDate>,
    ) -> Result<Option<NaiveDate>, SchedulerError> {
        let local_now = now_utc.with_timezone(&self.timezone);
        let date = local_now.date_naive();

        if last_attempted_date == Some(date) {
            return Ok(None);
        }

        if now_utc >= self.scheduled_utc_for_date(date)? {
            Ok(Some(date))
        } else {
            Ok(None)
        }
    }

    pub fn next_fire_after(&self, now_utc: DateTime<Utc>) -> Result<DateTime<Utc>, SchedulerError> {
        let local_now = now_utc.with_timezone(&self.timezone);
        let date = local_now.date_naive();
        let today_fire = self.scheduled_utc_for_date(date)?;

        if now_utc < today_fire {
            return Ok(today_fire);
        }

        let next_date =
            date.checked_add_days(Days::new(1))
                .ok_or_else(|| SchedulerError::InvalidSchedule {
                    reason: "could not compute next schedule date".to_owned(),
                })?;
        self.scheduled_utc_for_date(next_date)
    }

    pub fn duration_until_next_fire(
        &self,
        now_utc: DateTime<Utc>,
    ) -> Result<Duration, SchedulerError> {
        let next_fire = self.next_fire_after(now_utc)?;
        let duration = next_fire
            .signed_duration_since(now_utc)
            .to_std()
            .unwrap_or_else(|_| Duration::from_secs(1));

        Ok(duration.max(Duration::from_secs(1)))
    }

    pub fn scheduled_utc_for_date(
        &self,
        date: NaiveDate,
    ) -> Result<DateTime<Utc>, SchedulerError> {
        for offset in 0..=MAX_DST_GAP_MINUTES {
            let local_time = date.and_time(self.time) + ChronoDuration::minutes(offset);
            match self.timezone.from_local_datetime(&local_time) {
                LocalResult::Single(instant) => return Ok(instant.with_timezone(&Utc)),
                LocalResult::Ambiguous(earliest, latest) => {
                    return Ok(earliest.min(latest).with_timezone(&Utc));
                }
                LocalResult::None => {}
            }
        }

        Err(SchedulerError::InvalidSchedule {
            reason: format!(
                "configured local time {} never occurs on {} in {}",
                self.time, date, self.timezone
            ),
        })
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DailyRunOutcome {
    Started { date: NaiveDate, run_id: uuid::Uuid },
    Resumed { date: NaiveDate, run_id: uuid::Uuid },
    SkippedAlreadyComplete { date: NaiveDate, run_id: uuid::Uuid },
    SkippedAlreadyRunning { date: NaiveDate },
}

#[derive(Debug)]
pub enum SchedulerError {
    InvalidScheduleTime { value: String },
    InvalidTimezone { value: String },
    InvalidSchedule { reason: String },
    Database(sqlx::Error),
    Pipeline(PipelineError),
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScheduleTime { value } => {
                write!(formatter, "invalid scheduler time {value:?}; expected HH:MM")
            }
            Self::InvalidTimezone { value } => {
                write!(formatter, "invalid scheduler timezone {value:?}")
            }
            Self::InvalidSchedule { reason } => {
                write!(formatter, "invalid scheduler configuration: {reason}")
            }
            Self::Database(_) => write!(formatter, "scheduler database operation failed"),
            Self::Pipeline(error) => write!(formatter, "scheduled pipeline failed: {error}"),
        }
    }
}

impl Error for SchedulerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Pipeline(error) => Some(error),
            Self::InvalidScheduleTime { .. }
            | Self::InvalidTimezone { .. }
            | Self::InvalidSchedule { .. } => None,
        }
    }
}

impl From<PipelineError> for SchedulerError {
    fn from(error: PipelineError) -> Self {
        Self::Pipeline(error)
    }
}

struct DateLock {
    connection: PoolConnection<Postgres>,
    date: NaiveDate,
    lock_key: i32,
}

impl DateLock {
    async fn release(mut self) -> Result<(), SchedulerError> {
        let released = sqlx::query_scalar::<_, bool>("SELECT pg_advisory_unlock($1, $2)")
            .bind(SCHEDULER_LOCK_CLASS)
            .bind(self.lock_key)
            .fetch_one(&mut *self.connection)
            .await
            .map_err(SchedulerError::Database)?;

        if released {
            Ok(())
        } else {
            Err(SchedulerError::InvalidSchedule {
                reason: format!("scheduler lock for {} was not held", self.date),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc_schedule(time: &str) -> DailySchedule {
        DailySchedule::from_config(&SchedulerConfig {
            time: time.to_owned(),
            timezone: "UTC".to_owned(),
        })
        .expect("valid UTC schedule")
    }

    #[test]
    fn parses_configured_time_and_timezone() {
        let schedule = DailySchedule::from_config(&SchedulerConfig {
            time: "09:30".to_owned(),
            timezone: "America/New_York".to_owned(),
        })
        .expect("valid schedule");

        assert_eq!(
            schedule.time,
            NaiveTime::from_hms_opt(9, 30, 0).expect("time")
        );
        assert_eq!(schedule.timezone, chrono_tz::America::New_York);
    }

    #[test]
    fn rejects_invalid_timezone() {
        let error = DailySchedule::from_config(&SchedulerConfig {
            time: "09:30".to_owned(),
            timezone: "No/SuchZone".to_owned(),
        })
        .expect_err("invalid timezone should fail");

        assert!(matches!(error, SchedulerError::InvalidTimezone { .. }));
    }

    #[test]
    fn due_date_is_none_before_scheduled_time() {
        let schedule = utc_schedule("09:30");
        let now = Utc
            .with_ymd_and_hms(2026, 7, 1, 9, 29, 59)
            .single()
            .expect("valid time");

        assert_eq!(schedule.due_date_at(now, None).expect("due date"), None);
    }

    #[test]
    fn due_date_is_today_at_or_after_scheduled_time() {
        let schedule = utc_schedule("09:30");
        let now = Utc
            .with_ymd_and_hms(2026, 7, 1, 9, 30, 0)
            .single()
            .expect("valid time");

        assert_eq!(
            schedule.due_date_at(now, None).expect("due date"),
            Some(NaiveDate::from_ymd_opt(2026, 7, 1).expect("date"))
        );
    }

    #[test]
    fn due_date_does_not_repeat_for_last_attempted_date() {
        let schedule = utc_schedule("09:30");
        let now = Utc
            .with_ymd_and_hms(2026, 7, 1, 12, 0, 0)
            .single()
            .expect("valid time");
        let date = NaiveDate::from_ymd_opt(2026, 7, 1).expect("date");

        assert_eq!(
            schedule.due_date_at(now, Some(date)).expect("due date"),
            None
        );
    }

    #[test]
    fn next_fire_moves_to_tomorrow_after_todays_time_passed() {
        let schedule = utc_schedule("09:30");
        let now = Utc
            .with_ymd_and_hms(2026, 7, 1, 12, 0, 0)
            .single()
            .expect("valid time");
        let next = schedule.next_fire_after(now).expect("next fire");

        assert_eq!(
            next,
            Utc.with_ymd_and_hms(2026, 7, 2, 9, 30, 0)
                .single()
                .expect("valid time")
        );
    }
}
