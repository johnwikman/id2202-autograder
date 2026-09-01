// Connection and schema modification utilities

use chrono::{DateTime, TimeDelta, Utc};
use diesel::{
    self, Connection, ExpressionMethods, OptionalExtension, PgConnection, QueryDsl, RunQueryDsl,
    SelectableHelper,
};
use std::collections::BTreeSet;

use crate::{
    config::Settings,
    db::models::{
        raw::{NewSubmissionJobRow, NewSubmissionRow, SubmissionRow},
        ClaimResult, JobSpec, JobStatus, RegisterResult, StoredOriginKind, Submission,
        SubmissionJobPlain,
    },
    error::Error,
    reporting::Report,
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
            log::error!("Failed to connect to database: {e:#}");
            Error::auto_msg("failed to connect to database", e)
        })?;

        log::debug!("Connection established.");
        Ok(DatabaseConnection { conn })
    }

    /// Notifies all listeners on the channel `ch`. This does not include any
    /// payload in the notification.
    ///
    /// Warning: The value for `ch` can never come from a user as that will be
    /// hardcoded into the query.
    ///
    /// See this link for more information about `NOTIFY`:
    /// https://www.postgresql.org/docs/current/sql-notify.html
    pub fn notify<S: AsRef<str>>(&mut self, ch: S) -> Result<(), Error> {
        // Check that the channel is only ASCII alphabet chars
        if !ch.as_ref().bytes().all(|c| c.is_ascii_alphabetic()) {
            return Error::err_format("notify channel", ch.as_ref());
        }

        diesel::sql_query(format!("NOTIFY {};", ch.as_ref())).execute(&mut self.conn).map_err(
            |e| Error::auto_msg(format!("could not notify channel \"{}\"", ch.as_ref()), e),
        )?;
        Ok(())
    }

    /// Registers an incoming submission, applying each tag's throttle policy to
    /// the jobs before they are inserted.
    ///
    /// The returned submission carries the jobs as they were recorded, so the
    /// caller can see which of them were deferred and which were rejected.
    ///
    /// ## Warning about Race Conditions
    /// This may return an error if two threads at the same time attempt to
    /// register a submission from the same origin that has not previously been
    /// registered in the database. This depends on the implementation of
    /// `K::resolve(conn, origin)` for the specific origin kind.
    pub fn register_submission<K: StoredOriginKind>(
        &mut self,
        requested_tags: &[&str],
        mut jobs: Vec<JobSpec<'_>>,
        origin: &K::NewOriginRow,
        user: &str,
        commit: &str,
    ) -> Result<RegisterResult, Error> {
        use crate::db::schema::{
            submission_jobs,
            submission_origins::{self, columns as src_col},
            submissions,
        };
        use diesel::sql_types::{Array, BigInt, Integer, Nullable, Text, Timestamptz};

        let (submission_id, superseded_ids) = self.conn.transaction(|conn| {
            let now = Utc::now();

            let (src, kind_src) = K::resolve(conn, origin)?;

            // Lock the row in submission_origins for this origin using
            // `.for_update()`.
            //
            // We do not care about any returned value from this query and just
            // want to lock the row so nothing else can work with this origin
            // during this transation.
            submission_origins::table
                .select(src_col::id)
                .filter(src_col::id.eq(src.id))
                .for_update()
                .first::<i64>(conn)
                .map_err(|e: diesel::result::Error| {
                    Error::auto_msg(format!("could not lock submission origin {}", src.id), e)
                })?;

            // Any pending jobs for this origin that shares a tag with those
            // that this submission also wants to grade is superseded.
            //
            // IMPORTANT: The superseding has to be done before checking for
            // throttled jobs, such that any job that would be superseded does
            // not count towards the rate_limit/budget.
            /// A pending job that the submission being registered replaced.
            #[derive(diesel::QueryableByName)]
            struct SupersededJob {
                #[diesel(sql_type = BigInt)]
                id: i64,
            }

            // Supersede anything jobs with equal tags that are still waiting
            // to be run.
            let tags: Vec<&str> = jobs.iter().map(|job| job.tag.name.as_str()).collect();
            let superseded: Vec<SupersededJob> = diesel::sql_query(
                "
                UPDATE submission_jobs j
                SET status_code = $3,
                    status_text = $4,
                    voided_at   = now()
                WHERE j.tag = ANY($2)
                  AND j.id IN (SELECT p.id FROM v_pending_jobs p WHERE p.origin_id = $1)
                RETURNING j.id
                ",
            )
            .bind::<BigInt, _>(src.id)
            .bind::<Array<Text>, _>(&tags)
            .bind::<Integer, _>(JobStatus::Superseded as i32)
            .bind::<Text, _>(JobStatus::Superseded.to_string())
            .get_results(conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg(format!("could not supersede the jobs of origin {}", src.id), e)
            })?;

            // Perform throttling checks for each job
            for job in jobs.iter_mut() {
                let tag = job.tag;
                if !tag.rate_limit.enable && !tag.budget.enable {
                    // For efficiency, don't check job for tags with throttling
                    // disabled.
                    continue;
                }

                #[derive(diesel::QueryableByName)]
                struct ThrottlingCheck {
                    /// How much of a tag's budget the origin has spent.
                    #[diesel(sql_type = BigInt)]
                    budget_used: i64,
                    /// When the n'th most recent run of this tag (from this
                    /// origin) was eligible for grading.
                    #[diesel(sql_type = Nullable<Timestamptz>)]
                    nth: Option<DateTime<Utc>>,
                }

                // The `j.voided_at IS NULL` ensures that any voided jobs
                // (superseded, cancelled, etc.) do not count toward the
                // throttling check.
                let chk: ThrottlingCheck = diesel::sql_query(
                    "
                    WITH prior AS (
                        SELECT COALESCE(j.eligible_at, s.submitted_at) AS eligible_at
                        FROM submission_jobs j JOIN submissions s ON j.submission_id = s.id
                        WHERE s.origin_id = $1 AND j.tag = $2 AND j.voided_at IS NULL
                    )
                    SELECT (SELECT count(*) FROM prior) AS budget_used,
                           (SELECT eligible_at FROM prior
                            ORDER BY eligible_at DESC LIMIT 1 OFFSET $3 - 1) AS nth
                    ",
                )
                .bind::<BigInt, _>(src.id)
                .bind::<Text, _>(&tag.name)
                .bind::<BigInt, _>(i64::from(tag.rate_limit.n.max(1)))
                .get_result(conn)
                .map_err(|e: diesel::result::Error| {
                    Error::auto_msg(format!("could not probe the throttle of tag {}", tag.name), e)
                })?;

                if tag.budget.enable && chk.budget_used >= i64::from(tag.budget.max_runs) {
                    // Note: This will also set the job in the implementation
                    // of `JobSpec::to_row`.
                    job.status = JobStatus::Rejected;
                    continue;
                }

                if tag.rate_limit.enable {
                    let window = TimeDelta::seconds(tag.rate_limit.window_seconds as i64);

                    job.eligible_at = Some(chk.nth.map_or(now, |nth| now.max(nth + window)));
                }
            }

            let sub: SubmissionRow = diesel::insert_into(submissions::table)
                .values(NewSubmissionRow {
                    submitted_at: now,
                    requested_tags: requested_tags.iter().map(|s| s.to_string()).collect(),
                    origin_id: src.id,
                    report: None,
                })
                .returning(SubmissionRow::as_returning())
                .get_result(conn)
                .map_err(|e: diesel::result::Error| {
                    Error::auto_msg("could not insert new submission into database", e)
                })?;

            K::insert_info(conn, &sub, &kind_src, user, commit)?;

            // Inserted with `eligible_at` already decided, so no runner can
            // claim a job before its throttle has been applied.
            let job_rows: Vec<NewSubmissionJobRow> =
                jobs.iter().map(|spec| spec.to_row(sub.id)).collect();
            diesel::insert_into(submission_jobs::table).values(&job_rows).execute(conn).map_err(
                |e: diesel::result::Error| {
                    Error::auto_msg(
                        format!("could not insert the jobs of submission {}", sub.id),
                        e,
                    )
                },
            )?;

            let superseded_ids: Vec<i64> = superseded.iter().map(|old| old.id).collect();
            Ok::<_, Error>((sub.id, superseded_ids))
        })?;

        Ok(RegisterResult {
            submission: Submission::by_id(self, submission_id)?,
            superseded: Submission::by_job_ids(self, &superseded_ids)?,
        })
    }

    /// Records a submission that produced no jobs, keeping `report` as the
    /// explanation of why nothing could be graded.
    ///
    /// ## Warning about Race Conditions
    /// This may return an error if two threads at the same time attempt to
    /// register a submission from the same origin that has not previously been
    /// registered in the database. This depends on the implementation of
    /// `K::resolve(conn, origin)` for the specific origin kind.
    pub fn register_ungradable_submission<K: StoredOriginKind>(
        &mut self,
        requested_tags: &[&str],
        report: &Report,
        origin: &K::NewOriginRow,
        user: &str,
        commit: &str,
    ) -> Result<Submission, Error> {
        use crate::db::schema::submissions;

        let stored_report = serde_json::to_value(report)?;

        let submission_id = self.conn.transaction(|conn| {
            let (src, kind_src) = K::resolve(conn, origin)?;

            // No lock is taken on the origin: writing no jobs means the
            // throttle history is neither read nor extended.
            let sub: SubmissionRow = diesel::insert_into(submissions::table)
                .values(NewSubmissionRow {
                    submitted_at: Utc::now(),
                    requested_tags: requested_tags.iter().map(|s| s.to_string()).collect(),
                    origin_id: src.id,
                    report: Some(stored_report),
                })
                .returning(SubmissionRow::as_returning())
                .get_result(conn)
                .map_err(|e: diesel::result::Error| {
                    Error::auto_msg("could not insert new submission into database", e)
                })?;

            K::insert_info(conn, &sub, &kind_src, user, commit)?;

            Ok::<_, Error>(sub.id)
        })?;

        Submission::by_id(self, submission_id)
    }

    /// Tries to claim a wave of jobs for the runner with the specified ID,
    /// returning `None` if there was nothing to claim.
    pub fn try_claim_submission(&mut self, runner_id: i32) -> Result<Option<ClaimResult>, Error> {
        use diesel::sql_types::{BigInt, Integer};
        use diesel::QueryableByName;

        #[derive(QueryableByName)]
        struct OriginId {
            #[diesel(sql_type = BigInt)]
            id: i64,
        }

        /// Identifies the claimed jobs. The rest of their columns come from
        /// reading the submission back afterwards.
        #[derive(QueryableByName)]
        struct ClaimedJob {
            #[diesel(sql_type = BigInt)]
            id: i64,
            #[diesel(sql_type = BigInt)]
            submission_id: i64,
        }

        let claimed: Vec<ClaimedJob> = self
            .conn
            .transaction(|conn| {
                // Step 1: Pick a submission origin that has pending work (that
                // is eligible to be claimed) and nothing that is currently
                // being run. We use the two views set up in the diesel
                // migration to help with this. Then lock that origin for the
                // rest of this transaction.
                //
                // The locking is done with FOR UPDATE.
                //
                // We use SKIP LOCKED to handle the case of two concurrent
                // queries, such that if a submission origin is locked, then we
                // can simply skip over that origin (without blocking) and see
                // if there are other origins available.
                let picked: Option<OriginId> = diesel::sql_query(
                    "
                    SELECT src.id
                    FROM submission_origins src
                    WHERE EXISTS     (SELECT 1 FROM v_claimable_jobs c WHERE c.origin_id = src.id)
                      AND NOT EXISTS (SELECT 1 FROM v_active_jobs    a WHERE a.origin_id = src.id)
                    -- Ensure we get the origin with the lowest available submission id.
                    ORDER BY (
                            SELECT min(c.submission_id)
                            FROM v_claimable_jobs c
                            WHERE c.origin_id = src.id
                    )
                    LIMIT 1
                    FOR UPDATE OF src SKIP LOCKED
                ",
                )
                .get_result(conn)
                .optional()?;

                let Some(OriginId { id: origin_id }) = picked else {
                    // Nothing to claim
                    return Ok(Vec::new());
                };

                // TODO: The query above may be optimized such as to not have
                // to rediscover the submission below.

                // Step 2: At this point, the origin with `origin_id` is locked
                // to this transaction and we have exclusive access to it.
                //
                // Now claim all eligible jobs on the first submission (ordered
                // by submission id) from the locked origin which has eligible
                // jobs.
                diesel::sql_query(
                    "
                    UPDATE submission_jobs j
                    SET assigned_runner_id = $2
                    WHERE j.id IN (
                            SELECT c.id
                            FROM v_claimable_jobs c
                            WHERE c.submission_id = (
                                SELECT min(c2.submission_id)
                                FROM v_claimable_jobs c2
                                WHERE c2.origin_id = $1
                            )
                          )
                      AND NOT EXISTS (SELECT 1 FROM v_active_jobs a WHERE a.origin_id = $1)
                    RETURNING j.id, j.submission_id
                ",
                )
                .bind::<BigInt, _>(origin_id)
                .bind::<Integer, _>(runner_id)
                .get_results(conn)
            })
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg(format!("error claiming jobs for runner {runner_id}"), e)
            })?;

        let Some(first) = claimed.first() else {
            return Ok(None);
        };

        // The jobs have been claimed, now extract the data. Note that an error
        // at this point will cause the submission to be stale.

        let submission: Submission = Submission::by_id(self, first.submission_id)?;
        let claimed_ids: BTreeSet<i64> = claimed.iter().map(|c| c.id).collect();
        let (claimed_jobs, rest): (Vec<SubmissionJobPlain>, Vec<SubmissionJobPlain>) =
            submission.jobs.into_iter().partition(|j| claimed_ids.contains(&j.id));

        Ok(Some(ClaimResult {
            claimed: Submission { jobs: claimed_jobs, ..submission },
            deferred: rest.into_iter().filter(|j| j.terminal_at().is_none()).collect(),
        }))
    }
}
