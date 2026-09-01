//! Database models.
//!
//! `raw` holds the row structs, which mirror `schema.rs`. The other modules
//! are aggregates the rest of the program works with. The aggregates also
//! carry the queries that read and write from the tables they represent.

pub mod origin;
pub mod raw;
pub mod submission;

pub use origin::{StoredOriginKind, StoredOriginKindID, SubmissionOrigin};
pub use raw::{
    NewSubmissionOriginGitHubRow, NewSubmissionOriginGitLabRow, SubmissionInfoGitHubRow,
    SubmissionInfoGitLabRow, SubmissionOriginGitHubRow, SubmissionOriginGitLabRow,
    SubmissionOriginRow,
};
pub use submission::{
    ClaimResult, JobSpec, JobStatus, RegisterResult, Submission, SubmissionJob, SubmissionJobPlain,
    SubmissionJobWithReport, SubmissionStatus, SubmissionWithReports,
};
