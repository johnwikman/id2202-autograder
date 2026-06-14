// Connection and schema modification utilities

use diesel::{
    self, Connection, ExpressionMethods, OptionalExtension, PgConnection, QueryDsl, RunQueryDsl,
    SelectableHelper,
};
use num_traits::FromPrimitive;
use rand::Rng;
use std::time::SystemTime;

use crate::{
    config::Settings,
    db::models::{
        NewSubmission, Submission, SubmissionInfo, SubmissionInfoGitHub, SubmissionInfoGitLab,
        SubmissionSource, SubmissionSourceGitHub, SubmissionSourceGitLab, SubmissionSourceKind,
        SubmissionStatusCode,
    },
    error::Error,
    github, gitlab,
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
        Ok(DatabaseConnection { conn: conn })
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

        diesel::sql_query(&format!("NOTIFY {};", ch.as_ref()))
            .execute(&mut self.conn)
            .map_err(|e| {
                Error::auto_msg(format!("could not notify channel \"{}\"", ch.as_ref()), e)
            })?;
        Ok(())
    }

    /// Registers an incoming GitHub submission in the database.
    ///
    /// ## Warning about Race Conditions
    /// This may return an error if two threads attempt to register a
    /// submission with the same `domain`, `org`, and `repo` values at the same
    /// time.
    pub fn register_github_submission(
        &mut self,
        grading_tags: &Vec<&str>,
        domain: &str,
        user: &str,
        org: &str,
        repo: &str,
        ssh_url: &str,
        commit: &str,
    ) -> Result<i64, Error> {
        use crate::db::{
            models::{NewSubmissionInfoGitHub, SubmissionSourceGitHub},
            schema::{submission_info_github, submissions},
        };

        // Make sure that this happens as a single transaction, unrolling it on
        // an error.
        //
        // TODO:
        // Fix the race condition where two threads are inserting at the same
        // time, one inserts and the other gets None back. The one who gets
        // None back will select, but cannot find the submission source since
        // that has not yet been created. Somehow needs to do both inserts
        // atomically or lock the table somehow.
        //
        // The consequence of this is that this function will return an error,
        // but the database will remain in good health. So it is not fatal from
        // a server perspective, but it is an annoying edge case.
        let (src, gh_src) = self.conn.transaction(|conn| {
            use crate::db::{
                models::{
                    NewSubmissionSource, NewSubmissionSourceGitHub,
                },
                schema::{
                    submission_source_github::{self, columns as ghsrc_col},
                    submission_sources::{self, columns as src_col},
                },
            };
            let ghsrc_insert_check = diesel::insert_into(submission_source_github::table)
                .values(NewSubmissionSourceGitHub {
                    domain: domain.to_string(),
                    org: org.to_string(),
                    repo: repo.to_string(),
                    ssh_url: ssh_url.to_string(),
                })
                .on_conflict_do_nothing()
                .returning(SubmissionSourceGitHub::as_returning())
                .get_result(conn)
                .optional()?;

            if let Some(new_gh_src) = ghsrc_insert_check {
                // Inserted a new row into the GitHub submissions, so need to
                // insert a row into the submission_source table too. Also
                // generate a random auth_key for this source.
                let mut key: Vec<u8> = vec![0u8; 32];
                rand::rng().fill_bytes(key.as_mut_slice());

                let src = diesel::insert_into(submission_sources::table)
                    .values(NewSubmissionSource {
                        kind: SubmissionSourceKind::GitHub as i32,
                        kind_id: new_gh_src.id,
                        auth_key: bs58::encode(key).into_string(),
                    })
                    .returning(SubmissionSource::as_returning())
                    .get_result(conn)
                    .inspect_err(|e: &diesel::result::Error| {
                        log::error!(
                                "Could not insert a submission source for GitHub source id {}: {}",
                                new_gh_src.id,
                            e,
                        )
                    })?;

                Ok::<_, diesel::result::Error>((src, new_gh_src))
            } else {
                let gh_src = submission_source_github::table
                    .select(SubmissionSourceGitHub::as_select())
                    .filter(ghsrc_col::domain.eq(domain))
                    .filter(ghsrc_col::org.eq(org))
                    .filter(ghsrc_col::repo.eq(repo))
                    .first(conn).inspect_err(|e: &diesel::result::Error| {log::error!("Expected to find an existing GitHub source in the database with {} {} {}: {}", domain, org, repo, e)})?;

                let src = submission_sources::table
                    .select(SubmissionSource::as_select())
                    .filter(src_col::kind.eq(SubmissionSourceKind::GitHub as i32))
                    .filter(src_col::kind_id.eq(gh_src.id))
                    .first(conn)
                    .inspect_err(|e: &diesel::result::Error| {
                        log::error!("Expected to find a submission source referencing GitHub source with id {}: {}", gh_src.id, e)
                    })?;

                Ok((src, gh_src))
            }
        })?;

        // Atomically execute the insert into the database, ensuring that we
        // roll back both the submission and source info on failure.
        self.conn.transaction(|conn| {
            let sub: Submission = diesel::insert_into(submissions::table)
                .values(NewSubmission {
                    date_submitted: std::time::SystemTime::now(),
                    grading_tags: grading_tags.join(";"),
                    exec_finished: false,
                    exec_status_code: SubmissionStatusCode::NotStarted as i32,
                    source_id: src.id,
                })
                .returning(Submission::as_returning())
                .get_result(conn)
                .map_err(|e: diesel::result::Error| {
                    log::error!("Could not insert new submission into database: {e}");
                    Error::auto_msg("could not insert new submission into database", e)
                })?;

            diesel::insert_into(submission_info_github::table)
                .values(NewSubmissionInfoGitHub {
                    submission_id: sub.id,
                    github_source_id: gh_src.id,
                    commit: commit.to_string(),
                    user: user.to_string(),
                })
                .execute(conn)
                .map_err(|e: diesel::result::Error| {
                    log::error!("Could not insert GitHub info into database: {e}");
                    Error::auto_msg("could not insert new submission into database", e)
                })?;

            Ok(sub.id)
        })
    }

    /// Registers an incoming GitLab submission in the database.
    ///
    /// See `register_github_submission` for more details.
    pub fn register_gitlab_submission(
        &mut self,
        grading_tags: &Vec<&str>,
        domain: &str,
        user: &str,
        namespace: &str,
        repo: &str,
        ssh_url: &str,
        commit: &str,
    ) -> Result<i64, Error> {
        use crate::db::{
            models::{NewSubmissionInfoGitLab, SubmissionSourceGitLab},
            schema::{submission_info_gitlab, submissions},
        };

        let (src, gh_src) = self.conn.transaction(|conn| {
                use crate::db::{
                    models::{
                        NewSubmissionSource, NewSubmissionSourceGitLab,
                    },
                    schema::{
                        submission_source_gitlab::{self, columns as glsrc_col},
                        submission_sources::{self, columns as src_col},
                    },
                };
                let glsrc_insert_check = diesel::insert_into(submission_source_gitlab::table)
                    .values(NewSubmissionSourceGitLab {
                        domain: domain.to_string(),
                        namespace: namespace.to_string(),
                        repo: repo.to_string(),
                        ssh_url: ssh_url.to_string(),
                    })
                    .on_conflict_do_nothing()
                    .returning(SubmissionSourceGitLab::as_returning())
                    .get_result(conn)
                    .optional()?;

                if let Some(new_gl_src) = glsrc_insert_check {
                    // Inserted a new row into the GitHub submissions, so need to
                    // insert a row into the submission_source table too. Also
                    // generate a random auth_key for this source.
                    let mut key: Vec<u8> = vec![0u8; 32];
                    rand::rng().fill_bytes(key.as_mut_slice());

                    let src = diesel::insert_into(submission_sources::table)
                        .values(NewSubmissionSource {
                            kind: SubmissionSourceKind::GitLab as i32,
                            kind_id: new_gl_src.id,
                            auth_key: bs58::encode(key).into_string(),
                        })
                        .returning(SubmissionSource::as_returning())
                        .get_result(conn)
                        .inspect_err(|e: &diesel::result::Error| {
                            log::error!(
                                    "Could not insert a submission source for GitHub source id {}: {}",
                                    new_gl_src.id,
                                e,
                            )
                        })?;

                    Ok::<_, diesel::result::Error>((src, new_gl_src))
                } else {
                    let gl_src = submission_source_gitlab::table
                        .select(SubmissionSourceGitLab::as_select())
                        .filter(glsrc_col::domain.eq(domain))
                        .filter(glsrc_col::namespace.eq(namespace))
                        .filter(glsrc_col::repo.eq(repo))
                        .first(conn).inspect_err(|e: &diesel::result::Error| {log::error!("Expected to find an existing GitLab source in the database with {} {} {}: {}", domain, namespace, repo, e)})?;

                    let src = submission_sources::table
                        .select(SubmissionSource::as_select())
                        .filter(src_col::kind.eq(SubmissionSourceKind::GitLab as i32))
                        .filter(src_col::kind_id.eq(gl_src.id))
                        .first(conn)
                        .inspect_err(|e: &diesel::result::Error| {
                            log::error!("Expected to find a submission source referencing GitLab source with id {}: {}", gl_src.id, e)
                        })?;

                    Ok((src, gl_src))
                }
            })?;

        self.conn.transaction(|conn| {
            let sub: Submission = diesel::insert_into(submissions::table)
                .values(NewSubmission {
                    date_submitted: std::time::SystemTime::now(),
                    grading_tags: grading_tags.join(";"),
                    exec_finished: false,
                    exec_status_code: SubmissionStatusCode::NotStarted as i32,
                    source_id: src.id,
                })
                .returning(Submission::as_returning())
                .get_result(conn)
                .map_err(|e: diesel::result::Error| {
                    log::error!("Could not insert new submission into database: {e}");
                    Error::auto_msg("could not insert new submission into database", e)
                })?;

            diesel::insert_into(submission_info_gitlab::table)
                .values(NewSubmissionInfoGitLab {
                    submission_id: sub.id,
                    gitlab_source_id: gh_src.id,
                    commit: commit.to_string(),
                    user: user.to_string(),
                })
                .execute(conn)
                .map_err(|e: diesel::result::Error| {
                    log::error!("Could not insert GitLab info into database: {e}");
                    Error::auto_msg("could not insert new submission into database", e)
                })?;

            Ok(sub.id)
        })
    }

    /// Returns all information submission with the specified submission id.
    pub fn get_submission_info(&mut self, sub_id: i64) -> Result<SubmissionInfo, Error> {
        use crate::db::schema::{
            submission_info_github::{self, columns as ghinfo_col},
            submission_info_gitlab::{self, columns as glinfo_col},
            submission_source_github::{self, columns as ghsrc_col},
            submission_source_gitlab::{self, columns as glsrc_col},
            submission_sources::{self, columns as subsrc_col},
            submissions::{self, columns as sub_col},
        };

        // TODO: Here we should do a join, instead 4 separate queries...

        let sub: Submission = submissions::table
            .select(Submission::as_select())
            .filter(sub_col::id.eq(sub_id))
            .first(&mut self.conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg(
                    format!("could not get submission {sub_id} from database"),
                    e,
                )
            })?;

        let src: SubmissionSource = submission_sources::table
            .select(SubmissionSource::as_select())
            .filter(subsrc_col::id.eq(sub.source_id))
            .first(&mut self.conn)?;

        match SubmissionSourceKind::from_i32(src.kind) {
            Some(SubmissionSourceKind::GitHub) => {
                let gh_src = submission_source_github::table
                    .select(SubmissionSourceGitHub::as_select())
                    .filter(ghsrc_col::id.eq(src.kind_id))
                    .first(&mut self.conn)?;

                let gh_info = submission_info_github::table
                    .select(SubmissionInfoGitHub::as_select())
                    .filter(ghinfo_col::submission_id.eq(sub.id))
                    .filter(ghinfo_col::github_source_id.eq(gh_src.id))
                    .first(&mut self.conn)?;

                Ok(SubmissionInfo::GitHub {
                    sub: sub,
                    src: src,
                    gh_src: gh_src,
                    gh_info: gh_info,
                })
            }
            Some(SubmissionSourceKind::GitLab) => {
                let gl_src = submission_source_gitlab::table
                    .select(SubmissionSourceGitLab::as_select())
                    .filter(glsrc_col::id.eq(src.kind_id))
                    .first(&mut self.conn)?;

                let gl_info = submission_info_gitlab::table
                    .select(SubmissionInfoGitLab::as_select())
                    .filter(glinfo_col::submission_id.eq(sub.id))
                    .filter(glinfo_col::gitlab_source_id.eq(gl_src.id))
                    .first(&mut self.conn)?;

                Ok(SubmissionInfo::GitLab {
                    sub: sub,
                    src: src,
                    gl_src: gl_src,
                    gl_info: gl_info,
                })
            }
            None => Error::err_runtime(format!(
                "Invalid source kind {} for submission source with id {}",
                src.kind, src.id
            )),
        }
    }

    /// Return all the queued submissions in the database, oldest first
    pub fn queued_submissions(&mut self) -> Result<Vec<Submission>, Error> {
        use crate::db::schema::submissions::{
            self, assigned_runner_id, date_submitted, exec_finished,
        };

        let ret: Vec<Submission> = submissions::table
            .select(Submission::as_select())
            .filter(exec_finished.eq(false))
            .filter(assigned_runner_id.is_null())
            .order(date_submitted.asc())
            .load(&mut self.conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg("could not get queued submissions from database", e)
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
                    s2.assigned_runner_id IS NULL
                AND
                  -- Make sure that something from this source isn't already being graded.
                  s2.source_id NOT IN (
                      SELECT s3.source_id FROM submissions AS s3
                      WHERE
                          s3.exec_finished = false
                      AND
                          s3.assigned_runner_id IS NOT NULL
                      FOR UPDATE
                  )
                ORDER BY date_submitted ASC
                LIMIT 1
                -- This below is important to ensure that is gets executed
                -- atomically.
                FOR UPDATE
            )
            UPDATE submissions s
            SET assigned_runner_id = {runner_id}
              FROM queued_entries AS qe
              WHERE s.id = qe.id

            RETURNING *;
            "
        ))
        .get_results(&mut self.conn)
        .map_err(|e: diesel::result::Error| {
            Error::auto_msg(
                format!("error assigning a submission to runner {runner_id}"),
                e,
            )
        })?;

        // This should always return some by this stage...
        Ok(assigned_subs.get(0).map(|s| s.to_owned()))
    }

    /// Returns the submissions that are currently being handled by the runner
    /// with the provided runner id.
    pub fn active_submissions(&mut self, runner_id: i32) -> Result<Vec<Submission>, Error> {
        use crate::db::schema::submissions::{
            self, assigned_runner_id, date_submitted, exec_finished,
        };

        let ret: Vec<Submission> = submissions::table
            .select(Submission::as_select())
            .filter(exec_finished.eq(false))
            .filter(assigned_runner_id.eq(runner_id))
            .order(date_submitted.asc())
            .load(&mut self.conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg(
                    format!("could not get active submissions for runner {runner_id}"),
                    e,
                )
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
                Error::auto_msg(
                    format!("Could not set exec_date_started for submission {submission_id}"),
                    e,
                )
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
                Error::auto_msg(
                    format!("Could not set exec_date_finished for submission {submission_id}"),
                    e,
                )
            })
    }

    /// Updates the entry in the database, and also sends a report back to the
    /// submission source. The format of the sent report depends on the kind of
    /// source.
    pub fn report_and_status(
        &mut self,
        settings: &Settings,
        info: &SubmissionInfo,
        report: &Report,
        status: SubmissionStatusCode,
        exec_finished: bool,
    ) -> Result<(), Error> {
        use crate::db::{models::SubmissionStatusCode as SSC, schema::submissions};

        // async is annoying when you don't need it...
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| Error::auto_msg("could not unwrap tokio runtime", e))?;

        match info {
            SubmissionInfo::GitHub {
                sub,
                src: _,
                gh_src,
                gh_info,
            } => {
                use crate::github::CommitState as GHCS;

                log::debug!("Setting commit information for submission {}", sub.id);
                let (domain, org, repo, commit) =
                    (&gh_src.domain, &gh_src.org, &gh_src.repo, &gh_info.commit);

                if let Some(instance) = settings
                    .submission
                    .github
                    .known_instances
                    .iter()
                    .find(|ki| ki.domain == *domain)
                {
                    rt.block_on(async {
                        github::create_commit_message(
                            settings,
                            instance,
                            org,
                            repo,
                            commit,
                            &report.to_markdown(&settings.reporting.markdown),
                        )
                        .await
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
                        | SSC::TestCasesTimedOut
                        | SSC::OutputLimitExceeded
                        | SSC::SubmissionTimedOut => GHCS::Failure,
                        SSC::AutograderFailure => GHCS::Failure,
                    };

                    rt.block_on(async {
                        github::create_commit_status(
                            settings, instance, org, repo, commit, gh_state, None,
                        )
                        .await
                    })
                    .unwrap_or_else(|e| {
                        log::warn!("Could not set status for commit {commit} on {repo}: {e}");
                    });
                } else {
                    log::warn!("Could not set statis for commit {commit}: No GitHub instance configured for domain {domain}.");
                }
            }
            SubmissionInfo::GitLab {
                sub,
                src: _,
                gl_src,
                gl_info,
            } => {
                use crate::gitlab::CommitState as GLCS;

                log::debug!("Setting commit information for submission {}", sub.id);
                let (domain, namespace, repo, commit) = (
                    &gl_src.domain,
                    &gl_src.namespace,
                    &gl_src.repo,
                    &gl_info.commit,
                );

                if let Some(instance) = settings
                    .submission
                    .gitlab
                    .known_instances
                    .iter()
                    .find(|ki| ki.domain == *domain)
                {
                    rt.block_on(async {
                        gitlab::create_commit_message(
                            settings,
                            instance,
                            namespace,
                            repo,
                            commit,
                            &report.to_markdown(&settings.reporting.markdown),
                        )
                        .await
                    })
                    .unwrap_or_else(|e| {
                        log::warn!("Could not create message for commit {commit} on {repo}: {e}");
                    });

                    let gl_state: gitlab::CommitState = match status {
                        SSC::NotStarted => GLCS::Pending,
                        SSC::Running => GLCS::Running,
                        SSC::Success => GLCS::Success,
                        SSC::SubmissionError
                        | SSC::BuildError
                        | SSC::BuildTimedOut
                        | SSC::TestCasesFailed
                        | SSC::TestCasesTimedOut
                        | SSC::OutputLimitExceeded
                        | SSC::SubmissionTimedOut => GLCS::Failed,
                        SSC::AutograderFailure => GLCS::Canceled,
                    };

                    rt.block_on(async {
                        gitlab::set_commit_status(
                            settings, instance, namespace, repo, commit, gl_state, None,
                        )
                        .await
                    })
                    .unwrap_or_else(|e| {
                        log::warn!("Could not set status for commit {commit} on {repo}: {e}");
                    });
                } else {
                    log::warn!("Could not set statis for commit {commit}: No GitLab instance configured for domain {domain}.");
                }
            }
        }

        let sub = info.get_submission();

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
                submissions::exec_report.eq(serde_json::to_value(report)?),
            ))
            .execute(&mut self.conn)
            .map(|_| ())
            .map_err(|e| {
                Error::auto_msg(
                    format!(
                        "could not set status to {:?} for submission {}",
                        status, sub.id
                    ),
                    e,
                )
            })
    }

    /// Just set the status of the submission in the database, without sending
    /// a report to the submission source.
    pub fn set_status(
        &mut self,
        sub: &Submission,
        status: SubmissionStatusCode,
        exec_finished: bool,
    ) -> Result<(), Error> {
        use crate::db::schema::submissions;

        diesel::update(submissions::table)
            .filter(submissions::id.eq(sub.id))
            .set((
                submissions::exec_status_code.eq(status as i32),
                submissions::exec_finished.eq(exec_finished),
            ))
            .execute(&mut self.conn)
            .map(|_| ())
            .map_err(|e| {
                Error::auto_msg(
                    format!(
                        "could not set status to {:?} for submission {}",
                        status, sub.id
                    ),
                    e,
                )
            })
    }
}
