//! Aggregate models for submissions and submission jobs.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use diesel::{
    self,
    backend::Backend,
    deserialize::{self, FromSql},
    pg::Pg,
    prelude::{Queryable, QueryableByName, Selectable},
    serialize::{self, Output, ToSql},
    sql_types::Integer,
    AsExpression, ExpressionMethods, FromSqlRow, QueryDsl, RunQueryDsl, SelectableHelper,
};
use itertools::Itertools;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::FromPrimitive;
use serde::Deserialize;

use super::origin::SubmissionOrigin;
use super::raw::{NewSubmissionJobRow, SubmissionRow};
use crate::{
    config::Tag, db::conn::DatabaseConnection, error::Error, error_if_not_eq, reporting::Report,
};

/// A submission has no status of its own. This is derived from its jobs and its
/// report by `Submission::status`. **For display purposes only.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionStatus {
    /// Queued or throttled. Which one is per job, via `eligible_at`.
    Waiting,
    InProgress,
    Success,
    Failed,
    /// Every job was voided. The submission finished without ever running.
    Aborted,
    /// Nothing the submission carries says what became of it.
    Unknown,
}

impl std::fmt::Display for SubmissionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Waiting => write!(f, "Waiting"),
            Self::InProgress => write!(f, "In progress"),
            Self::Success => write!(f, "Success"),
            Self::Failed => write!(f, "Failed"),
            Self::Aborted => write!(f, "Aborted"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

/// A registered submission and anything that it may have superseded.
#[derive(Debug, Clone)]
pub struct RegisterResult {
    /// A read-back of the submission that was registered in the database.
    pub submission: Submission,

    /// The submissions whose pending jobs this one replaced, each paired with
    /// the jobs that were replaced by this registration.
    ///
    /// Each submission carries all of its jobs, so its derived status is the
    /// whole submission's rather than the replaced part's.
    pub superseded: Vec<(Submission, Vec<SubmissionJobPlain>)>,
}

/// A claimed submission. This contains the submission that was `claimed` and
/// any `deferred` submission jobs that could not be claimed with that
/// submission. The `claimed` submission struct will not contain any deferred
/// jobs.
#[derive(Debug, Clone)]
pub struct ClaimResult {
    pub claimed: Submission,
    pub deferred: Vec<SubmissionJobPlain>,
}

/// A submission, its jobs, and its origin.
///
/// The type parameter `J` is used to specify which information that a
/// submission job can carry. The default is `SubmissionJobPlain`, without any
/// generated reports. Use the `SubmissionWithReports` type alias to ensure
/// that all jobs also contain their grading report.
#[derive(Debug, Clone)]
pub struct Submission<J = SubmissionJobPlain> {
    pub id: i64,
    pub submitted_at: DateTime<Utc>,
    pub requested_tags: Vec<String>,
    pub jobs: Vec<J>,
    pub origin: SubmissionOrigin,

    /// Set only for exceptional circumstances, usually when no jobs could be
    /// created.
    pub report: Option<Report>,
}

/// A submission with any generated job reports included.
pub type SubmissionWithReports = Submission<SubmissionJobWithReport>;

impl<J: SubmissionJob> Submission<J> {
    /// The submission-wide status, derived from the jobs if there are any and
    /// from the submission report otherwise. For display purposes only.
    pub fn status(&self) -> SubmissionStatus {
        // Note: `SubmissionJob::plain` is a cheap operation.
        let jobs = || self.jobs.iter().map(SubmissionJob::plain);

        if self.jobs.is_empty() {
            return match &self.report {
                // The known places where the report can be set is on invalid
                // grading tags.
                Some(Report::InvalidTag(_)) => SubmissionStatus::Failed,
                // All other reports are not known to be set.
                _ => SubmissionStatus::Unknown,
            };
        }

        if jobs().any(|j| j.assigned_runner_id.is_some() && j.finished_at.is_none()) {
            return SubmissionStatus::InProgress;
        }
        if jobs().any(|j| j.terminal_at().is_none()) {
            return SubmissionStatus::Waiting;
        }
        if jobs().any(|j| j.status.is_error() && !j.status.is_voided()) {
            return SubmissionStatus::Failed;
        }
        if jobs().all(|j| j.status == JobStatus::Success) {
            return SubmissionStatus::Success;
        }
        // A voided job never ran, so it is not a grading failure. Only when
        // every job was voided does the submission read as aborted.
        if jobs().all(|j| j.status.is_voided()) {
            return SubmissionStatus::Aborted;
        }
        SubmissionStatus::Unknown
    }

    /// When work on the submission began, which is the earliest job start.
    /// `None` if no job has started.
    pub fn started_at(&self) -> Option<DateTime<Utc>> {
        self.jobs.iter().filter_map(|job| job.plain().started_at).min()
    }

    /// When the submission completed, which is when the latest job finished or
    /// was voided. Returns `None` while any job is still unfinished.
    pub fn finished_at(&self) -> Option<DateTime<Utc>> {
        let mut latest: Option<DateTime<Utc>> = None;
        for job in &self.jobs {
            let finished = job.plain().terminal_at()?;
            latest = Some(latest.map_or(finished, |l| l.max(finished)));
        }
        latest
    }

    /// Fetches all submissions of the specified `ids` from the database.
    ///
    /// # Warning
    /// This will not check that it fetches exactly all submissions specified
    /// by `ids`. That has to be checked by the caller.
    pub fn all_by_id(db: &mut DatabaseConnection, ids: &[i64]) -> Result<Vec<Self>, Error> {
        use crate::db::schema::submissions::{self, columns as sub_col};

        let rows: Vec<SubmissionRow> = submissions::table
            .select(SubmissionRow::as_select())
            .filter(sub_col::id.eq_any(ids))
            .load(&mut db.conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg("could not get the submissions from database", e)
            })?;

        Self::from_rows(db, rows)
    }

    /// The submission with the given id, or `None` if there is no such
    /// submission.
    pub fn by_id_opt(db: &mut DatabaseConnection, id: i64) -> Result<Option<Self>, Error> {
        Ok(Self::all_by_id(db, &[id])?.into_iter().next())
    }

    /// The submission with the given id. Return an error if there is no
    /// submission with that id.
    pub fn by_id(db: &mut DatabaseConnection, id: i64) -> Result<Self, Error> {
        Self::by_id_opt(db, id)?
            .ok_or_else(|| Error::runtime(format!("no submission with id {id}")))
    }

    /// Fetches the submissions that are responsible for the jobs whose ids are
    /// specified by `job_ids`. Each submission is returned as an entry in a
    /// vector of tuples:
    ///
    /// ```plain
    /// [
    ///   (sub1, "jobs in job_ids that sub1 is responsible for"),
    ///   (sub2, "jobs in job_ids that sub2 is responsible for"),
    ///   ...
    /// ]
    /// ```
    ///
    /// The returned vector is not guaranteed to follow any specific order.
    ///
    /// # Note
    /// The submission itself will carry all of its jobs, not just the ones
    /// from `job_ids`. Always check the second element in the tuple for the
    /// jobs that were actually requested.
    ///
    /// # Warning
    /// Any unmatched jobs in `job_ids` are silently dropped.
    pub fn by_job_ids(
        db: &mut DatabaseConnection,
        job_ids: &[i64],
    ) -> Result<Vec<(Self, Vec<SubmissionJobPlain>)>, Error> {
        use crate::db::schema::submission_jobs::{self, columns as job_col};

        if job_ids.is_empty() {
            return Ok(Vec::new());
        }

        let owners: Vec<(i64, i64)> = submission_jobs::table
            .select((job_col::submission_id, job_col::id))
            .filter(job_col::id.eq_any(job_ids))
            .load(&mut db.conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg("could not find the submissions of the jobs", e)
            })?;

        let mut named: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
        for (submission_id, job_id) in owners {
            named.entry(submission_id).or_default().insert(job_id);
        }

        let ids: Vec<i64> = named.keys().copied().collect();
        Ok(Self::all_by_id(db, &ids)?
            .into_iter()
            .map(|sub| {
                let wanted = named.get(&sub.id).cloned().unwrap_or_default();
                let jobs = sub
                    .jobs
                    .iter()
                    .map(SubmissionJob::plain)
                    .filter(|job| wanted.contains(&job.id))
                    .cloned()
                    .collect();
                (sub, jobs)
            })
            .collect())
    }

    /// Submissions with at least one unfinished job assigned to this runner.
    pub fn assigned_to_runner(
        db: &mut DatabaseConnection,
        runner_id: i32,
    ) -> Result<Vec<Self>, Error> {
        use crate::db::schema::submission_jobs::{self, columns as job_col};

        let ids: Vec<i64> = submission_jobs::table
            .select(job_col::submission_id)
            .filter(job_col::assigned_runner_id.eq(runner_id))
            .filter(job_col::finished_at.is_null())
            .distinct()
            .load(&mut db.conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg(format!("could not find jobs held by runner {runner_id}"), e)
            })?;

        Self::all_by_id(db, &ids)
    }

    /// Submissions with at least one unfinished job assigned to a runner id at
    /// or above `n_runners`.
    pub fn assigned_to_retired_runners(
        db: &mut DatabaseConnection,
        n_runners: usize,
    ) -> Result<Vec<Self>, Error> {
        use crate::db::schema::submission_jobs::{self, columns as job_col};

        // A runner id is an i32, so no id can be at or above a larger bound.
        let n_runners = i32::try_from(n_runners).unwrap_or(i32::MAX);

        let ids: Vec<i64> = submission_jobs::table
            .select(job_col::submission_id)
            .filter(job_col::assigned_runner_id.ge(n_runners))
            .filter(job_col::finished_at.is_null())
            .distinct()
            .load(&mut db.conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg("could not find jobs held by retired runners", e)
            })?;

        Self::all_by_id(db, &ids)
    }

    /// Constructs a Submission for each provided row, populating the jobs and
    /// origin fields of each row.
    pub fn from_rows(
        db: &mut DatabaseConnection,
        rows: Vec<SubmissionRow>,
    ) -> Result<Vec<Self>, Error> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let with_jobs = J::with_rows(db, rows)?;

        let ids: Vec<i64> = with_jobs.iter().map(|(row, _)| row.id).collect();
        let origins = SubmissionOrigin::of_submissions(db, &ids)?;

        error_if_not_eq!(ids.len(), origins.len());

        with_jobs
            .into_iter()
            .zip(origins)
            .map(|((row, jobs), origin)| {
                let report = match row.report {
                    Some(v) => Some(Report::deserialize(v).map_err(|e| {
                        Error::auto_msg(format!("bad report on submission {}", row.id), e)
                    })?),
                    None => None,
                };
                Ok(Submission {
                    submitted_at: row.submitted_at,
                    requested_tags: row.requested_tags,
                    jobs,
                    id: row.id,
                    origin,
                    report,
                })
            })
            .collect()
    }
}

// .-------------------------------------------------------------------------------.
// |  ____        _               _         _                   _       _          |
// | / ___| _   _| |__  _ __ ___ (_)___ ___(_) ___  _ __       | | ___ | |__  ___  |
// | \___ \| | | | '_ \| '_ ` _ \| / __/ __| |/ _ \| '_ \   _  | |/ _ \| '_ \/ __| |
// |  ___) | |_| | |_) | | | | | | \__ \__ \ | (_) | | | | | |_| | (_) | |_) \__ \ |
// | |____/ \__,_|_.__/|_| |_| |_|_|___/___/_|\___/|_| |_|  \___/ \___/|_.__/|___/ |
// '-------------------------------------------------------------------------------'

/// The status of a tag grading job. Cheekily using HTTP-like codes here,
/// although they have nothing to do with the HTTP protocol.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, FromPrimitive, ToPrimitive, AsExpression, FromSqlRow,
)]
#[diesel(sql_type = Integer)]
pub enum JobStatus {
    NotStarted = 0,
    Running = 102,

    Success = 200,

    /// Faulted at submission time.
    SubmissionError = 400,
    /// The job's own total deadline timed out the job.
    JobTimedOut = 408,
    /// Voided: replaced by a newer submission of the same tag.
    Superseded = 409,
    /// Voided: cancelled.
    Cancelled = 410,
    /// Voided: over budget, never ran.
    Rejected = 429,

    BuildError = 470,
    BuildTimedOut = 471,
    BuildOutputLimitExceeded = 472,

    TestCasesFailed = 480,
    TestCasesTimedOut = 481,
    TestOutputLimitExceeded = 482,

    /// Internal failure by the autograder. Note that this can also be set as
    /// a status for voided jobs, if it was voided due to an internal failure.
    AutograderFailure = 500,
}

impl JobStatus {
    /// Whether this status code indicates that a job has finished running.
    pub fn is_finished(&self) -> bool {
        (*self as i32) >= 200
    }

    /// Whether or not this status code indicates that is has finished without
    /// success.
    pub fn is_error(&self) -> bool {
        (*self as i32) >= 400
    }

    /// Whether or not this code signals an OK status, meaning that the status
    /// value is between 200 and 299 inclusive.
    pub fn is_ok(&self) -> bool {
        matches!(*self as i32, 200..=299)
    }

    /// Whether this job reached a terminal state without ever being graded.
    pub fn is_voided(&self) -> bool {
        matches!(self, Self::Superseded | Self::Cancelled | Self::Rejected)
    }
}

impl FromSql<Integer, Pg> for JobStatus {
    fn from_sql(bytes: <Pg as Backend>::RawValue<'_>) -> deserialize::Result<Self> {
        let code = i32::from_sql(bytes)?;
        Self::from_i32(code).ok_or_else(|| format!("unknown job status code {code}").into())
    }
}

impl ToSql<Integer, Pg> for JobStatus {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> serialize::Result {
        <i32 as ToSql<Integer, Pg>>::to_sql(&(*self as i32), &mut out.reborrow())
    }
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotStarted => write!(f, "Not Started"),
            Self::Running => write!(f, "Running"),
            Self::Success => write!(f, "Success"),
            Self::SubmissionError => write!(f, "Submission Error"),
            Self::JobTimedOut => write!(f, "Job Timed Out"),
            Self::Superseded => write!(f, "Superseded"),
            Self::Cancelled => write!(f, "Cancelled"),
            Self::Rejected => write!(f, "Rejected"),
            Self::BuildError => write!(f, "Build Error"),
            Self::BuildTimedOut => write!(f, "Build Timed Out"),
            Self::BuildOutputLimitExceeded => write!(f, "Build Output Limit Exceeded"),
            Self::TestCasesFailed => write!(f, "Test Cases Failed"),
            Self::TestCasesTimedOut => write!(f, "Test Cases Timed Out"),
            Self::TestOutputLimitExceeded => write!(f, "Test Output Limit Exceeded"),
            Self::AutograderFailure => write!(f, "Autograder Failure"),
        }
    }
}

/// A job to create alongside a new submission. Jobs that are born terminal,
/// such as an unknown tag or a broken test configuration, carry the failing
/// status here.
#[derive(Debug, Clone)]
pub struct JobSpec<'a> {
    pub tag: &'a Tag,
    pub requested_as: Vec<String>,
    pub status: JobStatus,
    pub eligible_at: Option<DateTime<Utc>>,
}

impl<'a> JobSpec<'a> {
    /// Instantiate a raw submission_jobs row based on the provided job spec.
    /// Derives the timestamp fields (whether they are `NULL` or `now()`) based
    /// on the status.
    ///
    /// # Note
    /// If the status code of the job spec indicates that it is finished, then
    /// the job will be declared voided immediately as the row is inserted.
    pub fn to_row(&self, submission_id: i64) -> NewSubmissionJobRow {
        let now = Utc::now();
        NewSubmissionJobRow {
            submission_id,
            tag: self.tag.name.clone(),
            requested_as: self.requested_as.clone(),
            eligible_at: self.eligible_at,
            // A job is inserted unclaimed, so it cannot have run. Anything
            // born terminal is therefore voided rather than finished.
            voided_at: self.status.is_finished().then_some(now),
            status_code: self.status as i32,
        }
    }
}

/// What a `Submission` carries for each of its jobs.
pub trait SubmissionJob: Sized {
    /// Each row paired with its jobs. A submission with no jobs gets an empty
    /// vector.
    fn with_rows(
        db: &mut DatabaseConnection,
        rows: Vec<SubmissionRow>,
    ) -> Result<Vec<(SubmissionRow, Vec<Self>)>, Error>;

    /// A plain representation of the job.
    fn plain(&self) -> &SubmissionJobPlain;
}

/// One grading run of one tag: the unit of work a runner claims. Always
/// reached through the submission it belongs to, so it carries no
/// `submission_id`.
///
/// # Note
/// This does not include the generated report, as that is often very large.
/// Use `SubmissionJobPlain::with_report` to fetch it.
#[derive(Debug, Clone, Queryable, Selectable, QueryableByName)]
#[diesel(table_name = crate::db::schema::submission_jobs)]
#[diesel(check_for_backend(Pg))]
pub struct SubmissionJobPlain {
    pub id: i64,
    pub tag: String,
    pub requested_as: Vec<String>,
    pub eligible_at: Option<DateTime<Utc>>,
    pub voided_at: Option<DateTime<Utc>>,
    pub assigned_runner_id: Option<i32>,
    #[diesel(column_name = status_code)]
    pub status: JobStatus,
    pub status_text: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl SubmissionJob for SubmissionJobPlain {
    fn with_rows(
        db: &mut DatabaseConnection,
        rows: Vec<SubmissionRow>,
    ) -> Result<Vec<(SubmissionRow, Vec<Self>)>, Error> {
        use crate::db::schema::submission_jobs::{self, columns as job_col};

        let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();

        // Ordering by `submission_id` is necessary for `chunk_by` to properly
        // group the jobs by submission.
        let mut grouped: BTreeMap<i64, Vec<Self>> = submission_jobs::table
            .select((job_col::submission_id, Self::as_select()))
            .order((job_col::submission_id.asc(), job_col::id.asc()))
            .filter(job_col::submission_id.eq_any(&ids))
            .load::<(i64, Self)>(&mut db.conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg("could not get the jobs of the submissions", e)
            })?
            .into_iter()
            .chunk_by(|(submission_id, _)| *submission_id)
            .into_iter()
            .map(|(submission_id, group)| (submission_id, group.map(|(_, job)| job).collect()))
            .collect();

        Ok(rows
            .into_iter()
            .map(|row| {
                let jobs = grouped.remove(&row.id).unwrap_or_default();
                (row, jobs)
            })
            .collect())
    }

    fn plain(&self) -> &SubmissionJobPlain {
        self
    }
}

impl SubmissionJobPlain {
    /// When the job reached a terminal state, whether by being graded or by
    /// being voided without ever running. `None` while it is still
    /// outstanding.
    pub fn terminal_at(&self) -> Option<DateTime<Utc>> {
        self.finished_at.or(self.voided_at)
    }

    /// Whether this job is assigned to `runner_id` and has not yet finished
    /// running.
    pub fn is_claimed_by(&self, runner_id: i32) -> bool {
        self.assigned_runner_id == Some(runner_id) && self.finished_at.is_none()
    }

    /// This job together with the report it produced (if it has written one).
    pub fn with_report(
        self,
        db: &mut DatabaseConnection,
    ) -> Result<SubmissionJobWithReport, Error> {
        use crate::db::schema::submission_jobs::{self, columns as job_col};

        let stored: Option<serde_json::Value> = submission_jobs::table
            .select(job_col::report)
            .filter(job_col::id.eq(self.id))
            .first(&mut db.conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg(format!("could not get the report of job {}", self.id), e)
            })?;

        let report = match stored {
            Some(v) => Some(
                Report::deserialize(v)
                    .map_err(|e| Error::auto_msg(format!("bad report on job {}", self.id), e))?,
            ),
            None => None,
        };

        Ok(SubmissionJobWithReport { job: self, report })
    }

    /// Marks the job as being graded, stamping `started_at` with the current
    /// time.
    pub fn set_as_started(&mut self, db: &mut DatabaseConnection) -> Result<(), Error> {
        use crate::db::schema::submission_jobs::{self, columns as job_col};

        let now = Utc::now();
        let status = JobStatus::Running;

        diesel::update(submission_jobs::table)
            .filter(job_col::id.eq(self.id))
            .set((
                job_col::status_code.eq(status as i32),
                job_col::status_text.eq(status.to_string()),
                job_col::started_at.eq(now),
            ))
            .execute(&mut db.conn)
            .map_err(|e| {
                Error::auto_msg(format!("could not mark job {} as started", self.id), e)
            })?;

        self.status = status;
        self.status_text = Some(status.to_string());
        self.started_at = Some(now);
        Ok(())
    }

    /// Marks the job terminal, storing the report it produced if there is one.
    pub fn set_as_finished(
        &mut self,
        db: &mut DatabaseConnection,
        status: JobStatus,
        report: Option<&Report>,
    ) -> Result<(), Error> {
        Self::set_all_as_finished([self], db, status, report)
    }

    /// Marks every job in `jobs` terminal, giving them all the same status and
    /// report, in a single statement. Does nothing if `jobs` yields an empty
    /// iterator.
    ///
    /// # Note
    /// This will return an error if trying to set a non-started job as
    /// finished. Use `set_all_as_voided` then instead.
    pub fn set_all_as_finished<'a>(
        jobs: impl IntoIterator<Item = &'a mut Self>,
        db: &mut DatabaseConnection,
        status: JobStatus,
        report: Option<&Report>,
    ) -> Result<(), Error> {
        use crate::db::schema::submission_jobs::{self, columns as job_col};

        let jobs: Vec<&mut Self> = jobs.into_iter().collect();
        if jobs.is_empty() {
            return Ok(());
        }

        let now = Utc::now();
        let stored = match report {
            Some(r) => Some(serde_json::to_value(r)?),
            None => None,
        };
        let ids: Vec<i64> = jobs.iter().map(|job| job.id).collect();

        if let Some(unstarted_job) = jobs.iter().find(|j| j.started_at.is_none()) {
            return Error::err_runtime(format!(
                "could not set unstarted job with ID {} as finished",
                unstarted_job.id,
            ));
        }

        diesel::update(submission_jobs::table)
            .filter(job_col::id.eq_any(&ids))
            .set((
                job_col::status_code.eq(status as i32),
                job_col::status_text.eq(status.to_string()),
                job_col::finished_at.eq(now),
                job_col::report.eq(&stored),
            ))
            .execute(&mut db.conn)
            .map_err(|e| {
                Error::auto_msg(format!("could not finish jobs {ids:?} with {status}"), e)
            })?;

        for job in jobs {
            job.status = status;
            job.status_text = Some(status.to_string());
            job.finished_at = Some(now);
        }

        Ok(())
    }

    /// Marks every job in `jobs` as voided. Any runner assignment and run
    /// timestamps are cleared, so a job can be voided too regardless of its
    /// running status. Does nothing if `jobs` is empty.
    pub fn set_all_as_voided<'a>(
        jobs: impl IntoIterator<Item = &'a mut Self>,
        db: &mut DatabaseConnection,
        status: JobStatus,
        report: Option<&Report>,
    ) -> Result<(), Error> {
        use crate::db::schema::submission_jobs::{self, columns as job_col};

        let mut jobs: Vec<&mut Self> = jobs.into_iter().collect();
        if jobs.is_empty() {
            return Ok(());
        }

        let now = Utc::now();
        let stored = match report {
            Some(r) => Some(serde_json::to_value(r)?),
            None => None,
        };
        let ids: Vec<i64> = jobs.iter().map(|job| job.id).collect();

        diesel::update(submission_jobs::table)
            .filter(job_col::id.eq_any(&ids))
            .set((
                job_col::status_code.eq(status as i32),
                job_col::status_text.eq(status.to_string()),
                job_col::voided_at.eq(now),
                job_col::assigned_runner_id.eq(None::<i32>),
                job_col::started_at.eq(None::<DateTime<Utc>>),
                job_col::finished_at.eq(None::<DateTime<Utc>>),
                job_col::report.eq(stored),
            ))
            .execute(&mut db.conn)
            .map_err(|e| {
                Error::auto_msg(format!("could not void jobs {ids:?} with {status}"), e)
            })?;

        for job in jobs.iter_mut() {
            job.status = status;
            job.status_text = Some(status.to_string());
            job.voided_at = Some(now);
            job.assigned_runner_id = None;
            job.started_at = None;
            job.finished_at = None;
        }
        Ok(())
    }

    /// Voids all `jobs` with `JobStatus::Cancelled`. This is performed
    /// unconditionally on the provided jobs.
    pub fn abandon_all<'j>(
        jobs: impl IntoIterator<Item = &'j mut Self>,
        db: &mut DatabaseConnection,
        report: &Report,
    ) -> Result<(), Error> {
        Self::set_all_as_voided(jobs, db, JobStatus::Cancelled, Some(report))
    }
}

/// A job together with the report it produced.
#[derive(Debug, Clone)]
pub struct SubmissionJobWithReport {
    pub job: SubmissionJobPlain,
    pub report: Option<Report>,
}

impl SubmissionJob for SubmissionJobWithReport {
    fn with_rows(
        db: &mut DatabaseConnection,
        rows: Vec<SubmissionRow>,
    ) -> Result<Vec<(SubmissionRow, Vec<Self>)>, Error> {
        use crate::db::schema::submission_jobs::{self, columns as job_col};

        let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();

        // Ordering by `submission_id` is necessary for `chunk_by` to properly
        // group the jobs by submission.
        let mut grouped: BTreeMap<i64, Vec<Self>> = submission_jobs::table
            .select((job_col::submission_id, SubmissionJobPlain::as_select(), job_col::report))
            .order((job_col::submission_id.asc(), job_col::id.asc()))
            .filter(job_col::submission_id.eq_any(&ids))
            .load::<(i64, SubmissionJobPlain, Option<serde_json::Value>)>(&mut db.conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg("could not get the jobs of the submissions", e)
            })?
            .into_iter()
            .map(|(submission_id, job, stored)| {
                let report = match stored {
                    Some(v) => Some(Report::deserialize(v).map_err(|e| {
                        Error::auto_msg(format!("bad report on job {}", job.id), e)
                    })?),
                    None => None,
                };
                Ok((submission_id, SubmissionJobWithReport { job, report }))
            })
            .collect::<Result<Vec<_>, Error>>()?
            .into_iter()
            .chunk_by(|(submission_id, _)| *submission_id)
            .into_iter()
            .map(|(submission_id, group)| (submission_id, group.map(|(_, job)| job).collect()))
            .collect();

        Ok(rows
            .into_iter()
            .map(|row| {
                let jobs = grouped.remove(&row.id).unwrap_or_default();
                (row, jobs)
            })
            .collect())
    }

    fn plain(&self) -> &SubmissionJobPlain {
        &self.job
    }
}
