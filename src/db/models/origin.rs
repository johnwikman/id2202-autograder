//! Aggregate models for the origin tables in the database. I.e. where a
//! submission came from.

use diesel::{
    prelude::{Queryable, Selectable},
    Connection, ExpressionMethods, JoinOnDsl, OptionalExtension, PgConnection, QueryDsl,
    RunQueryDsl, SelectableHelper,
};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::FromPrimitive;
use rand::Rng;

use std::collections::BTreeMap;

use super::raw::{
    NewSubmissionInfoGitHubRow, NewSubmissionInfoGitLabRow, NewSubmissionOriginGitHubRow,
    NewSubmissionOriginGitLabRow, NewSubmissionOriginRow, SubmissionInfoGitHubRow,
    SubmissionInfoGitLabRow, SubmissionOriginGitHubRow, SubmissionOriginGitLabRow,
    SubmissionOriginRow, SubmissionRow,
};
use super::submission::SubmissionStatus;
use crate::{
    config::Settings,
    db::conn::DatabaseConnection,
    error::Error,
    origin::{github::GitHub, gitlab::GitLab, Origin, OriginKind},
    reporting::MetaReport,
};

/// Submission Origin Kind
#[derive(Debug, Clone, Copy, FromPrimitive, ToPrimitive)]
pub enum StoredOriginKindID {
    GitHub = 0,
    GitLab = 1,
}

impl std::fmt::Display for StoredOriginKindID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GitHub => write!(f, "GitHub"),
            Self::GitLab => write!(f, "GitLab"),
        }
    }
}

impl StoredOriginKindID {
    /// Inserts the `submission_origins` row referring to an origin of this kind
    /// that was just inserted. This will also generate a random 256-bit
    /// `auth_key` for the origin.
    fn insert_origin_row(
        &self,
        conn: &mut PgConnection,
        kind_id: i64,
    ) -> Result<SubmissionOriginRow, Error> {
        use crate::db::schema::submission_origins;

        let mut key: Vec<u8> = vec![0u8; 32];
        rand::rng().fill_bytes(key.as_mut_slice());

        diesel::insert_into(submission_origins::table)
            .values(NewSubmissionOriginRow {
                kind: *self as i32,
                kind_id,
                auth_key: bs58::encode(key).into_string(),
            })
            .returning(SubmissionOriginRow::as_returning())
            .get_result(conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg(
                    format!("could not insert a submission origin for {self} origin id {kind_id}"),
                    e,
                )
            })
    }

    /// The `submission_origins` row referring to an origin of this kind that
    /// is already in the database.
    fn get_origin_row(
        &self,
        conn: &mut PgConnection,
        kind_id: i64,
    ) -> Result<SubmissionOriginRow, Error> {
        use crate::db::schema::submission_origins::{self, columns as src_col};

        submission_origins::table
            .select(SubmissionOriginRow::as_select())
            .filter(src_col::kind.eq(*self as i32))
            .filter(src_col::kind_id.eq(kind_id))
            .first(conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg(
                    format!("expected a submission origin referencing {self} origin id {kind_id}"),
                    e,
                )
            })
    }
}

#[derive(Debug, Clone)]
pub struct SubmissionOrigin {
    /// Holds the `auth_key` and the origin id, which are reachable without
    /// matching on the kind.
    pub src_row: SubmissionOriginRow,

    /// Information about the actual origin
    pub origin: StoredOriginEnum,
}

/// Enumeration over possible kinds of origins stored in the database.
#[derive(Debug, Clone)]
pub enum StoredOriginEnum {
    GitHub(StoredOrigin<GitHub>),
    GitLab(StoredOrigin<GitLab>),
}

/// Common information that an origin needs to provide.
pub trait StoredOriginKind: OriginKind + Sized {
    type OriginRow: Selectable<diesel::pg::Pg>;
    type InfoRow: Selectable<diesel::pg::Pg>;
    type NewOriginRow;

    const KIND_ID: StoredOriginKindID;

    /// Returns information on the origin based on the stored information in
    /// the database.
    fn as_origin(
        settings: &Settings,
        origin_row: &Self::OriginRow,
        info_row: &Self::InfoRow,
    ) -> Result<Origin<Self>, Error>;

    /// Get an origin submission state for this kind, based on the general
    /// submission status.
    fn status_to_state(status: SubmissionStatus) -> Self::SubmissionState;

    /// Resolves the origin row from the database, returning its corresponding
    /// `submission_origins` row as well. If a matching origin row was not
    /// found in the database, a row is inserted into the database and then
    /// returned.
    ///
    /// The `Self::NewOriginRow` type is used to provide both the information
    /// for lookup as well as for insertion.
    fn resolve(
        conn: &mut PgConnection,
        origin: &Self::NewOriginRow,
    ) -> Result<(SubmissionOriginRow, Self::OriginRow), Error>;

    /// Records which commit of which user a submission is.
    ///
    /// # Note
    /// This is currently hardcoded to a user and a commit, but will be changed
    /// in the future to handle more generic submission origins which are not
    /// git-based.
    fn insert_info(
        conn: &mut PgConnection,
        submission: &SubmissionRow,
        origin: &Self::OriginRow,
        user: &str,
        commit: &str,
    ) -> Result<(), Error>;
}

/// A submission origin of one kind.
#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct StoredOrigin<K: StoredOriginKind> {
    /// The origin of the submission.
    #[diesel(embed)]
    pub src: K::OriginRow,
    /// Information about the specific submission from this origin.
    #[diesel(embed)]
    pub info: K::InfoRow,
}

impl<K: StoredOriginKind> StoredOrigin<K> {
    fn as_origin(&self, settings: &Settings) -> Result<Origin<K>, Error> {
        K::as_origin(settings, &self.src, &self.info)
    }
}

impl StoredOriginKind for GitHub {
    type OriginRow = SubmissionOriginGitHubRow;
    type InfoRow = SubmissionInfoGitHubRow;
    type NewOriginRow = NewSubmissionOriginGitHubRow;
    const KIND_ID: StoredOriginKindID = StoredOriginKindID::GitHub;

    fn as_origin(
        settings: &Settings,
        origin_row: &Self::OriginRow,
        info_row: &Self::InfoRow,
    ) -> Result<Origin<Self>, Error> {
        let instance = settings
            .submission
            .github
            .known_instances
            .iter()
            .find(|i| i.domain == origin_row.domain)
            .ok_or_else(|| {
                Error::runtime(format!("Could not find settings for domain {}", origin_row.domain))
            })?;
        Ok(Origin {
            info: Self::Info {
                instance: instance.clone(),
                organization_name: origin_row.org.clone(),
                repo_name: origin_row.repo.clone(),
                commit_hash: info_row.commit.clone(),
            },
        })
    }

    fn status_to_state(status: SubmissionStatus) -> Self::SubmissionState {
        type SS = SubmissionStatus;
        use crate::origin::github::CommitState as CS;
        match status {
            SS::Waiting => CS::Pending,
            SS::InProgress => CS::Pending,
            SS::Success => CS::Success,
            SS::Failed => CS::Failure,
            SS::Aborted => CS::Error,
            SS::Unknown => CS::Error,
        }
    }

    fn resolve(
        conn: &mut PgConnection,
        origin: &NewSubmissionOriginGitHubRow,
    ) -> Result<(SubmissionOriginRow, SubmissionOriginGitHubRow), Error> {
        use crate::db::schema::submission_origin_github::{self, columns as gh_col};

        conn.transaction(|conn| {
            let inserted = diesel::insert_into(submission_origin_github::table)
                .values(origin)
                .on_conflict_do_nothing()
                .returning(SubmissionOriginGitHubRow::as_returning())
                .get_result(conn)
                .optional()
                .map_err(|e: diesel::result::Error| {
                    Error::auto_msg("could not insert a GitHub origin", e)
                })?;

            if let Some(gh_src) = inserted {
                let src = Self::KIND_ID.insert_origin_row(conn, gh_src.id)?;
                return Ok((src, gh_src));
            }

            let gh_src = submission_origin_github::table
                .select(SubmissionOriginGitHubRow::as_select())
                .filter(gh_col::domain.eq(&origin.domain))
                .filter(gh_col::org.eq(&origin.org))
                .filter(gh_col::repo.eq(&origin.repo))
                .first(conn)
                .map_err(|e: diesel::result::Error| {
                    Error::auto_msg(
                        format!(
                            "expected an existing GitHub origin {} {} {}",
                            origin.domain, origin.org, origin.repo
                        ),
                        e,
                    )
                })?;

            let src = Self::KIND_ID.get_origin_row(conn, gh_src.id)?;
            Ok((src, gh_src))
        })
    }

    fn insert_info(
        conn: &mut PgConnection,
        submission: &SubmissionRow,
        origin: &SubmissionOriginGitHubRow,
        user: &str,
        commit: &str,
    ) -> Result<(), Error> {
        use crate::db::schema::submission_info_github;

        diesel::insert_into(submission_info_github::table)
            .values(NewSubmissionInfoGitHubRow {
                submission_id: submission.id,
                github_origin_id: origin.id,
                commit: commit.to_string(),
                user: user.to_string(),
            })
            .execute(conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg("could not insert GitHub info into database", e)
            })?;
        Ok(())
    }
}

impl StoredOriginKind for GitLab {
    type OriginRow = SubmissionOriginGitLabRow;
    type InfoRow = SubmissionInfoGitLabRow;
    type NewOriginRow = NewSubmissionOriginGitLabRow;
    const KIND_ID: StoredOriginKindID = StoredOriginKindID::GitLab;

    fn as_origin(
        settings: &Settings,
        origin_row: &Self::OriginRow,
        info_row: &Self::InfoRow,
    ) -> Result<Origin<Self>, Error> {
        let instance = settings
            .submission
            .gitlab
            .known_instances
            .iter()
            .find(|i| i.domain == origin_row.domain)
            .ok_or_else(|| {
                Error::runtime(format!("Could not find settings for domain {}", origin_row.domain))
            })?;
        Ok(Origin {
            info: Self::Info {
                instance: instance.clone(),
                namespace: origin_row.namespace.clone(),
                repo_name: origin_row.repo.clone(),
                commit_hash: info_row.commit.clone(),
            },
        })
    }

    fn status_to_state(status: SubmissionStatus) -> Self::SubmissionState {
        type SS = SubmissionStatus;
        use crate::origin::gitlab::CommitState as CS;
        match status {
            SS::Waiting => CS::Pending,
            SS::InProgress => CS::Running,
            SS::Success => CS::Success,
            SS::Failed => CS::Failed,
            SS::Aborted => CS::Canceled,
            SS::Unknown => CS::Skipped,
        }
    }

    fn resolve(
        conn: &mut PgConnection,
        origin: &NewSubmissionOriginGitLabRow,
    ) -> Result<(SubmissionOriginRow, SubmissionOriginGitLabRow), Error> {
        use crate::db::schema::submission_origin_gitlab::{self, columns as gl_col};

        conn.transaction(|conn| {
            let inserted = diesel::insert_into(submission_origin_gitlab::table)
                .values(origin)
                .on_conflict_do_nothing()
                .returning(SubmissionOriginGitLabRow::as_returning())
                .get_result(conn)
                .optional()
                .map_err(|e: diesel::result::Error| {
                    Error::auto_msg("could not insert a GitLab origin", e)
                })?;

            if let Some(gl_src) = inserted {
                let src = Self::KIND_ID.insert_origin_row(conn, gl_src.id)?;
                return Ok((src, gl_src));
            }

            let gl_src = submission_origin_gitlab::table
                .select(SubmissionOriginGitLabRow::as_select())
                .filter(gl_col::domain.eq(&origin.domain))
                .filter(gl_col::namespace.eq(&origin.namespace))
                .filter(gl_col::repo.eq(&origin.repo))
                .first(conn)
                .map_err(|e: diesel::result::Error| {
                    Error::auto_msg(
                        format!(
                            "expected an existing GitLab origin {} {} {}",
                            origin.domain, origin.namespace, origin.repo
                        ),
                        e,
                    )
                })?;

            let src = Self::KIND_ID.get_origin_row(conn, gl_src.id)?;
            Ok((src, gl_src))
        })
    }

    fn insert_info(
        conn: &mut PgConnection,
        submission: &SubmissionRow,
        origin: &SubmissionOriginGitLabRow,
        user: &str,
        commit: &str,
    ) -> Result<(), Error> {
        use crate::db::schema::submission_info_gitlab;

        diesel::insert_into(submission_info_gitlab::table)
            .values(NewSubmissionInfoGitLabRow {
                submission_id: submission.id,
                gitlab_origin_id: origin.id,
                commit: commit.to_string(),
                user: user.to_string(),
            })
            .execute(conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg("could not insert GitLab info into database", e)
            })?;
        Ok(())
    }
}

impl SubmissionOrigin {
    /// The origin of each submission in `ids`, one entry per id and in the
    /// order given.
    ///
    /// Returns an error if a submission is missing, if it has no origin row,
    /// or if its origin has a `kind` value that is not a `StoredOriginKindID`.
    pub fn of_submissions(db: &mut DatabaseConnection, ids: &[i64]) -> Result<Vec<Self>, Error> {
        use crate::db::schema::{
            submission_info_github::{self, columns as ghinfo_col},
            submission_info_gitlab::{self, columns as glinfo_col},
            submission_origin_github, submission_origin_gitlab,
            submission_origins::{self, columns as subsrc_col},
            submissions::{self, columns as sub_col},
        };

        // All BTreeMaps below map `submission_id` to rows of its origin.
        let src_rows: BTreeMap<i64, SubmissionOriginRow> = submission_origins::table
            .inner_join(submissions::table.on(sub_col::origin_id.eq(subsrc_col::id)))
            .select((sub_col::id, SubmissionOriginRow::as_select()))
            .filter(sub_col::id.eq_any(ids))
            .load(&mut db.conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg("could not get the origins of the submissions", e)
            })?
            .into_iter()
            .collect();

        let github: BTreeMap<i64, StoredOrigin<GitHub>> = submission_info_github::table
            .inner_join(submission_origin_github::table)
            .select((ghinfo_col::submission_id, StoredOrigin::<GitHub>::as_select()))
            .filter(ghinfo_col::submission_id.eq_any(ids))
            .load(&mut db.conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg("could not get the GitHub origins of the submissions", e)
            })?
            .into_iter()
            .collect();

        let gitlab: BTreeMap<i64, StoredOrigin<GitLab>> = submission_info_gitlab::table
            .inner_join(submission_origin_gitlab::table)
            .select((glinfo_col::submission_id, StoredOrigin::<GitLab>::as_select()))
            .filter(glinfo_col::submission_id.eq_any(ids))
            .load(&mut db.conn)
            .map_err(|e: diesel::result::Error| {
                Error::auto_msg("could not get the GitLab origins of the submissions", e)
            })?
            .into_iter()
            .collect();

        ids.iter()
            .map(|id| {
                let src_row = src_rows
                    .get(id)
                    .cloned()
                    .ok_or_else(|| Error::runtime(format!("no submission with id {id}")))?;

                let origin = match StoredOriginKindID::from_i32(src_row.kind) {
                    Some(StoredOriginKindID::GitHub) => {
                        github.get(id).cloned().map(StoredOriginEnum::GitHub)
                    }
                    Some(StoredOriginKindID::GitLab) => {
                        gitlab.get(id).cloned().map(StoredOriginEnum::GitLab)
                    }
                    None => {
                        return Error::err_runtime(format!(
                            "Invalid origin kind {} for submission origin with id {}",
                            src_row.kind, src_row.id
                        ))
                    }
                }
                .ok_or_else(|| Error::runtime(format!("submission {id} has no origin row")))?;

                Ok(SubmissionOrigin { src_row, origin })
            })
            .collect()
    }

    /// Wrapper around `Origin::fetch_into`.
    pub fn fetch_into(&self, settings: &Settings, dir: &str) -> Result<(), Error> {
        match &self.origin {
            StoredOriginEnum::GitHub(o) => o.as_origin(settings)?.fetch_into(settings, dir),
            StoredOriginEnum::GitLab(o) => o.as_origin(settings)?.fetch_into(settings, dir),
        }
    }

    /// Wrapper around `Origin::set_state_and_report`.
    pub async fn set_status_and_report<'a>(
        &self,
        settings: &Settings,
        report: &MetaReport<'a>,
        status: SubmissionStatus,
    ) -> Result<(), Error> {
        match &self.origin {
            StoredOriginEnum::GitHub(o) => {
                o.as_origin(settings)?
                    .set_state_and_report(settings, report, &GitHub::status_to_state(status), None)
                    .await
            }
            StoredOriginEnum::GitLab(o) => {
                o.as_origin(settings)?
                    .set_state_and_report(settings, report, &GitLab::status_to_state(status), None)
                    .await
            }
        }
    }
}
