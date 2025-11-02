// Connection and schema modification utilities

use diesel::{
    self, Connection, ExpressionMethods, PgConnection, QueryDsl, RunQueryDsl, SelectableHelper,
};
use std::time::SystemTime;

use crate::{
    db::models::{NewGitHubSubmission, Submission, SubmissionStatusCode},
    error::Error,
    github,
    settings::Settings,
};

pub struct DatabaseConnection {
    pub conn: PgConnection,
}

impl DatabaseConnection {
    /// Connects to the database using the postgres settings
    pub fn connect(s: &Settings) -> Result<Self, Error> {
        let conn_string: String = format!(
            "host={} port={} user={} password={} dbname=autograder connect_timeout=10",
            s.postgres.host, s.postgres.port, s.postgres.user, s.postgres.password
        );

        log::debug!("Connecting to postgres database with \"{}\"", conn_string);
        let conn = PgConnection::establish(&conn_string).map_err(|e| {
            log::error!("Failed to connect to database: {e}");
            Error::from(e)
        })?;

        log::debug!("Connection established.");
        Ok(DatabaseConnection { conn: conn })
    }

    /// Registers an incoming GitHub submission in the database.
    pub fn register_github_submission(
        &mut self,
        s: &Settings,
        grading_tags: &Vec<String>,
        user: &str,
        org: &str,
        repo: &str,
        commit: &str,
    ) -> Result<i64, Error> {
        use crate::db::schema::submissions;

        let sub = NewGitHubSubmission {
            date_submitted: std::time::SystemTime::now(),
            grading_tags: grading_tags.join(";"),
            exec_finished: false,
            exec_status_code: SubmissionStatusCode::NotStarted as i32,
            github_address: s.github.address.clone(),
            github_org: org.to_string(),
            github_repo: repo.to_string(),
            github_user: user.to_string(),
            github_commit: commit.to_string(),
        };
        let ret: Submission = diesel::insert_into(submissions::table)
            .values(&sub)
            .returning(Submission::as_returning())
            .get_result(&mut self.conn)
            .map_err(|e: diesel::result::Error| {
                log::error!("Could not insert new submission into database: {e}");
                Error::from(e)
            })?;

        Ok(ret.id)
    }

    /// Return all the submissions in the database, sorted by submission date.
    ///
    /// Can optionally set a date using `since` for how far back to look. Can
    /// also limit the number of responses using the `limit` argument.
    pub fn get_submissions(
        &mut self,
        since: Option<&SystemTime>,
        limit: Option<i64>,
    ) -> Result<Vec<Submission>, Error> {
        use crate::db::schema::submissions::{self, date_submitted};

        let base_q = submissions::table
            .select(Submission::as_select())
            .order(date_submitted.asc());

        let q_result = match (since, limit) {
            (Some(t), Some(l)) => base_q
                .filter(date_submitted.ge(t))
                .limit(l)
                .load(&mut self.conn),
            (Some(t), None) => base_q.filter(date_submitted.ge(t)).load(&mut self.conn),
            (None, Some(l)) => base_q.limit(l).load(&mut self.conn),
            _ => base_q.load(&mut self.conn),
        };

        let ret: Vec<Submission> = q_result.map_err(|e: diesel::result::Error| {
            log::error!("Could not get submissions from database: {e}");
            Error::from(e)
        })?;

        Ok(ret)
    }

    /// Returns the submission with the specified submission id.
    pub fn get_submission(&mut self, sub_id: i64) -> Result<Submission, Error> {
        use crate::db::schema::submissions::{self, id};

        let ret: Submission = submissions::table
            .select(Submission::as_select())
            .filter(id.eq(sub_id))
            .first(&mut self.conn)
            .map_err(|e: diesel::result::Error| {
                log::error!("Could not get submission from database: {e}");
                Error::from(e)
            })?;

        Ok(ret)
    }

    /// Return all the queued submissions in the database, oldest first
    pub fn queued_submissions(&mut self) -> Result<Vec<Submission>, Error> {
        use crate::db::schema::submissions::{
            self, assigned_runner, date_submitted, exec_finished,
        };

        let ret: Vec<Submission> = submissions::table
            .select(Submission::as_select())
            .filter(exec_finished.eq(false))
            .filter(assigned_runner.is_null())
            .order(date_submitted.asc())
            .load(&mut self.conn)
            .map_err(|e: diesel::result::Error| {
                log::error!("Could not get queued submissions from database: {e}");
                Error::from(e)
            })?;

        Ok(ret)
    }

    /// Tries to assign a submission to the runner with the specified ID, if
    /// there are any queued submissions.
    ///
    /// Returns None if we could not assign a new submission to this runner.
    /// If there was a queued submission that was assigned to this specific
    /// runner ID, then the database is atomically updated and the ID assigned
    /// to this runner.
    pub fn try_assign_submission(&mut self, runner_id: i32) -> Result<Option<Submission>, Error> {
        // Using custom SQL here, don't know how to do this in Diesel directly...
        // The .bind function for safe queries does not appear to work either...
        // (Formatting a single int argument should be safe though.)
        // https://www.postgresql.org/docs/17/sql-update.html
        // https://www.postgresql.org/docs/17/sql-select.html
        // https://www.postgresql.org/docs/17/functions-conditional.html
        let assigned_subs: Vec<Submission> = diesel::sql_query(format!(
            "
            WITH queued_entries AS (
                SELECT s2.id FROM submissions AS s2
                WHERE
                    s2.exec_finished = false
                AND
                    s2.assigned_runner IS NULL
                AND
                    -- Make sure that the user isn't already being graded.
                    s2.github_repo NOT IN (
                        SELECT s3.github_repo FROM submissions AS s3
                        WHERE
                            s3.exec_finished = false
                        AND
                            s3.assigned_runner IS NOT NULL
                        FOR UPDATE
                    )
                ORDER BY date_submitted ASC
                LIMIT 1
                -- This below is important to ensure that is gets executed
                -- atomically.
                FOR UPDATE
            )
            UPDATE submissions s
            SET assigned_runner = {runner_id}
              FROM queued_entries AS qe
              WHERE s.id = qe.id

            RETURNING *;
            "
        ))
        .get_results(&mut self.conn)
        .map_err(|e: diesel::result::Error| {
            log::error!("Error updating the database: {e}");
            Error::from(e)
        })?;

        // This should always return some by this stage...
        Ok(assigned_subs.get(0).map(|s| s.to_owned()))
    }

    /// Returns the submissions that are currently being handled by the runner
    /// with the provided runner id.
    pub fn active_submissions(&mut self, runner_id: i32) -> Result<Vec<Submission>, Error> {
        use crate::db::schema::submissions::{
            self, assigned_runner, date_submitted, exec_finished,
        };

        let ret: Vec<Submission> = submissions::table
            .select(Submission::as_select())
            .filter(exec_finished.eq(false))
            .filter(assigned_runner.eq(runner_id))
            .order(date_submitted.asc())
            .load(&mut self.conn)
            .map_err(|e: diesel::result::Error| {
                log::error!("Could not get submissions from database: {e}");
                Error::from(e)
            })?;

        Ok(ret)
    }

    /// Sets the exec_date_started to the current time and date
    pub fn set_exec_date_started(&mut self, submission_id: i64) -> Result<(), Error> {
        use crate::db::schema::submissions;

        diesel::update(submissions::table)
            .filter(submissions::id.eq(submission_id))
            .set(submissions::exec_date_started.eq(SystemTime::now()))
            .execute(&mut self.conn)
            .map(|_| ())
            .map_err(|e| {
                log::error!(
                    "Could not set exec_date_started for submission {}: {}",
                    submission_id,
                    &e
                );
                Error::from(e)
            })
    }

    /// Sets the exec_date_finished to the current time and date
    pub fn set_exec_date_finished(&mut self, submission_id: i64) -> Result<(), Error> {
        use crate::db::schema::submissions;

        diesel::update(submissions::table)
            .filter(submissions::id.eq(submission_id))
            .set((
                submissions::exec_date_finished.eq(SystemTime::now()),
                submissions::exec_finished.eq(true),
            ))
            .execute(&mut self.conn)
            .map(|_| ())
            .map_err(|e| {
                log::error!(
                    "Could not set exec_date_finished for submission {}: {}",
                    submission_id,
                    &e
                );
                Error::from(e)
            })
    }

    /// Updates the entry in the database, and also writes a comment to the
    /// GitHub commit.
    pub fn github_comment_and_status(
        &mut self,
        settings: &Settings,
        sub: &Submission,
        msg: &str,
        status: SubmissionStatusCode,
        exec_finished: bool,
    ) -> Result<(), Error> {
        use crate::{
            db::{models::SubmissionStatusCode as SSC, schema::submissions},
            github::CommitState as GHCS,
        };

        log::debug!("Setting commit information for submission {}", sub.id);
        let (org, repo, commit) = (&sub.github_org, &sub.github_repo, &sub.github_commit);

        // async is annoying when you don't need it...
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| Error::from(format!("Could not unwrap tokio runtime: {e}")))?;

        rt.block_on(async {
            github::create_commit_message(settings, org, repo, commit, msg).await
        })
        .unwrap_or_else(|e| {
            log::warn!("Could not create message for commit {commit} on {repo}: {e}");
        });

        let gh_state: github::CommitState = match status {
            SSC::NotStarted | SSC::Running => GHCS::Pending,
            SSC::Success => GHCS::Success,
            SSC::SubmissionError
            | SSC::BuildError
            | SSC::BuildTimedOut
            | SSC::TestCasesFailed
            | SSC::TestCasesTimedOut => GHCS::Failure,
            SSC::AutograderFailure => GHCS::Failure,
        };

        rt.block_on(async {
            github::create_commit_status(settings, org, repo, commit, gh_state, None).await
        })
        .unwrap_or_else(|e| {
            log::warn!("Could not set status for commit {commit} on {repo}: {e}");
        });

        log::debug!(
            "Setting submission status to {:?} for submission {}",
            status,
            sub.id
        );

        diesel::update(submissions::table)
            .filter(submissions::id.eq(sub.id))
            .set((
                submissions::exec_status_code.eq(status as i32),
                submissions::exec_finished.eq(exec_finished),
            ))
            .execute(&mut self.conn)
            .map(|_| ())
            .map_err(|e| {
                log::error!(
                    "Could not set status to {:?} for submission {}: {}",
                    status,
                    sub.id,
                    &e
                );
                Error::from(e)
            })
    }
}
