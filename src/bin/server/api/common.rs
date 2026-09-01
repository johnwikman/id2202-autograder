use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Display,
};

use id2202_autograder::{
    config::{tag_match, Settings, Tests},
    db::models::{JobSpec, JobStatus, RegisterResult, Submission, SubmissionJobPlain},
    error::Error,
    reporting::{
        structured_text::{StructuredInline, StructuredParagraph},
        MetaReport, Report, ReportInvalidTag, ReportMessage, ReportWrapper,
    },
    utils::utc_string,
};
use itertools::Itertools;

/// Sets the status and posts a report to the origins of the submissions in
/// `reg.superseded`.
///
/// Will attempt to set status and report for every origin, returning an Error
/// if one or more attempts failed.
///
/// # Important
/// The origin status will be derived based on the statuses of the `jobs`
/// field in the submission itself, not the replaced jobs in the tuple
/// `(sub, replaced) in reg.superseded`.
pub async fn report_superseded(settings: &Settings, reg: &RegisterResult) -> Result<(), Error> {
    let mut errs: Vec<Box<dyn std::error::Error + Send + Sync>> = vec![];
    for (old, replaced) in &reg.superseded {
        let rep = MetaReport::Structured(StructuredParagraph::Many(vec![
            StructuredParagraph::plain(format!(
                "This submission has been replaced by a newer submission (ID {}). The \
                    following tags of this submission will not be graded:",
                reg.submission.id
            )),
            StructuredParagraph::Itemized(
                replaced.iter().map(|job| StructuredInline::inline_code(job.tag.clone())).collect(),
            ),
        ]));

        old.origin.set_status_and_report(settings, &rep, old.status()).await.unwrap_or_else(|e| {
            errs.push(
                Error::runtime(format!("could not report superseded submission {}", old.id))
                    .with_cause(e.into())
                    .into(),
            );
        });
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Error::err_multi_cause("error reporting superseded jobs", errs)
    }
}

/// The markdown comment posted when a submission has been accepted, naming the
/// tags that will not be graded and when the ones being held back become
/// eligible.
pub fn acceptance_message<'a>(sub: &'a Submission) -> StructuredParagraph<'a> {
    use id2202_autograder::reporting::structured_text::{
        StructuredInline as Inline, StructuredParagraph as Par,
    };

    // Only the names that differ from the tag itself are worth showing.
    let derived_from = |job: &SubmissionJobPlain| {
        let derivs: Vec<&String> = job.requested_as.iter().filter(|r| **r != job.tag).collect();
        match derivs.as_slice() {
            [] => None,
            _ => Some(Inline::sep_space(vec![
                "(Derived from".into(),
                Inline::sep_comma(
                    derivs
                        .into_iter()
                        .map(|s| Inline::inline_code(Inline::plain(s.clone())))
                        .collect(),
                ),
                ")".into(),
            ])),
        }
    };

    let head = Inline::bold(Inline::Sep {
        sep: "",
        parts: vec![
            Inline::PlainStr("[Submission ID: "),
            Inline::Plain(sub.id.to_string()),
            Inline::PlainStr(" | "),
            Inline::sep_comma(
                sub.requested_tags.iter().map(|s| Inline::inline_code(s.clone())).collect(),
            ),
            Inline::PlainStr("]"),
        ],
    });

    let rejected: Vec<&SubmissionJobPlain> =
        sub.jobs.iter().filter(|j| j.status == JobStatus::Rejected).collect();

    // A NULL `eligible_at` means no throttle policy was applied to the job.
    let deferred: Vec<&SubmissionJobPlain> = sub
        .jobs
        .iter()
        .filter(|j| j.status != JobStatus::Rejected && j.eligible_at.is_some())
        .collect();

    let middle = if !sub.jobs.is_empty() && rejected.len() == sub.jobs.len() {
        "The autograder has received your submission, but none of the requested tags \
         will be graded."
    } else {
        "The autograder has successfully received your submission and will start grading \
         as soon as a runner is available. Additional information and results of your \
         submission will be provided as comments here."
    };

    let mut paragraphs = vec![Par::Paragraph(head), middle.into()];

    if !rejected.is_empty() {
        paragraphs.push(
            "The following tags will not be graded, because their grading budget has been \
             used up:"
                .into(),
        );
        paragraphs.push(Par::Itemized(
            rejected
                .iter()
                .map(|job| {
                    if let Some(deriv) = derived_from(job) {
                        Inline::sep_space(vec![Inline::inline_code(job.tag.clone()), deriv])
                    } else {
                        Inline::inline_code(job.tag.clone())
                    }
                })
                .collect(),
        ));
    }

    if !deferred.is_empty() {
        paragraphs.push(
            "The following tags are rate-limited, and will not be graded earlier than the \
             times given:"
                .into(),
        );
        paragraphs.push(Par::Itemized(
            deferred
                .iter()
                .filter_map(|job| job.eligible_at.as_ref().map(|e| (*job, e)))
                .map(|(job, eligible_at)| Inline::Sep {
                    sep: "",
                    parts: vec![
                        if let Some(deriv) = derived_from(job) {
                            Inline::sep_space(vec![Inline::inline_code(job.tag.clone()), deriv])
                        } else {
                            Inline::inline_code(job.tag.clone())
                        },
                        ": ".into(),
                        utc_string(eligible_at).into(),
                    ],
                })
                .collect(),
        ));
    }

    Par::Many(paragraphs)
}

/// Extracts grading tags from the the string in `from`. Returns a vector (that
/// may be empty) containing all grading tags on success. En error report is
/// returned on failure.
pub fn extract_grading_tags<'a>(
    settings: &Settings,
    from: &'a str,
) -> Result<Vec<&'a str>, Box<Report>> {
    // Check for grading tags. First adding them to BTreeSet to remove any
    // duplicates unique, then converting the set back to a vector.
    let mut grading_tag_set: BTreeSet<&str> = BTreeSet::new();
    let mut s: &'a str = from;
    while !s.is_empty() {
        // We split at i + 1 because we are interested in the string that
        // follows the tag symbol.
        if let Some((_, s_after)) = s.find(['#', '%']).and_then(|i| s.split_at_checked(i + 1)) {
            let (s_tag, s_rest) = tag_match(s_after);
            grading_tag_set.insert(s_tag);
            s = s_rest;
        } else {
            break;
        }
    }

    let grading_tags: Vec<&'a str> =
        grading_tag_set.into_iter().filter(|s| !s.is_empty()).collect();

    let tag_length =
        grading_tags.iter().map(|s| s.len()).reduce(|acc, e| acc + e).unwrap_or(0usize);

    if tag_length >= settings.submission.max_tag_length {
        Err(Box::new(Report::Wrapper(ReportWrapper {
            title: Some("Submission Error".to_string()),
            reports: vec![
                Report::Message(ReportMessage { msg: format!(
                    "The provided grading tags {} exceed the limit of {} characters. Your submission will not be graded.",
                    grading_tags.iter().format_with(", ", |t, f| f(&format_args!("`{t}`"))),
                    settings.submission.max_tag_length,
                )})
            ]
        })))
    } else {
        Ok(grading_tags)
    }
}

/// Expands the requested tags into the jobs to create, deduplicating tags that
/// several requested names resolve to and recording which names those were.
///
/// An unknown tag fails the whole submission, so this returns the report to
/// show the student rather than a partial job list.
pub fn resolve_jobs<'a>(
    tests: &'a Tests,
    requested: &[&str],
) -> Result<Vec<JobSpec<'a>>, Box<Report>> {
    // maps "actual tag" -> "the tags that it was derived from"
    let mut by_tag: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();

    for name in requested {
        let Some(tagnames) = tests.tag_resolution.get(*name) else {
            log::info!("Received invalid tag {name}");
            return Err(Box::new(Report::InvalidTag(ReportInvalidTag {
                tag_name: name.to_string(),
                known_grading_tags: tests.tags.keys().cloned().collect(),
                known_tag_groups: tests
                    .tag_resolution
                    .iter()
                    .filter(|(k, _)| !tests.tags.contains_key(*k))
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
                    .collect(),
            })));
        };
        for tagname in tagnames {
            by_tag.entry(tagname).or_default().insert(name.to_string());
        }
    }

    by_tag
        .into_iter()
        .map(|(tagname, requested_as)| {
            let Some(tag) = tests.tags.get(tagname) else {
                log::error!(
                    "Tag resolution resolved {tagname}, but the test \
                     configuration has no such tag. This is an critical error \
                     in the internal configuration handling."
                );
                return Err(Box::new(internal_error_report()));
            };
            Ok(JobSpec {
                tag,
                requested_as: requested_as.into_iter().collect(),
                status: JobStatus::NotStarted,
                eligible_at: None,
            })
        })
        .collect()
}

/// The report shown to a student when the autograder itself failed. Says
/// nothing about why: the cause is staff-internal and belongs in the log, which
/// is where the caller must put it.
pub fn internal_error_report() -> Report {
    Report::Message(ReportMessage {
        msg: "An internal error occurred while handling your submission. \
              Please contact course staff."
            .to_string(),
    })
}

pub enum RejectionReason<'a> {
    InvalidGroup { group: &'a str },
    InvalidRepoPrefix { repo: &'a str },
    InvalidRepoSuffix { repo: &'a str },
    ProhibitedRepoPrefix { repo: &'a str, prefix: &'a str },
    ProhibitedRepoSuffix { repo: &'a str, suffix: &'a str },
}

impl<'a> Display for RejectionReason<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidGroup { group } => {
                write!(f, "Invalid group {group}")
            }
            Self::InvalidRepoPrefix { repo } => {
                write!(f, "Invalid prefix for repository {repo}")
            }
            Self::InvalidRepoSuffix { repo } => {
                write!(f, "Invalid suffix for repository {repo}")
            }
            Self::ProhibitedRepoPrefix { repo, prefix } => {
                write!(f, "Prohibited prefix \"{prefix}\" for repository {repo}")
            }
            Self::ProhibitedRepoSuffix { repo, suffix } => {
                write!(f, "Prohibited suffix \"{suffix}\" for repository {repo}")
            }
        }
    }
}

/// Validates that a repository submitted for grading satisfies the prefix and
/// suffix criteria if specified. An empty list of criteria is ignored.
///
/// On error, the reason for rejection
pub fn validate_repo_prefix_suffix<'a>(
    group: &'a str,
    repository: &'a str,
    allowed_groups: &'a [String],
    allowed_repo_prefixes: &'a [String],
    allowed_repo_suffixes: &'a [String],
    prohibited_repo_prefixes: &'a [String],
    prohibited_repo_suffixes: &'a [String],
) -> Result<(), RejectionReason<'a>> {
    if !allowed_groups.is_empty() && !allowed_groups.iter().any(|org| org == group) {
        return Err(RejectionReason::InvalidGroup { group });
    }

    if !allowed_repo_prefixes.is_empty() {
        let allowed_prefix = allowed_repo_prefixes.iter().any(|pfx| repository.starts_with(pfx));
        if !allowed_prefix {
            return Err(RejectionReason::InvalidRepoPrefix { repo: repository });
        }
    }
    if !allowed_repo_suffixes.is_empty() {
        let allowed_suffix = allowed_repo_suffixes.iter().any(|sfx| repository.ends_with(sfx));
        if !allowed_suffix {
            return Err(RejectionReason::InvalidRepoSuffix { repo: repository });
        }
    }

    if let Some(prohibited_prefix) =
        prohibited_repo_prefixes.iter().find(|pfx| repository.starts_with(pfx.as_str()))
    {
        return Err(RejectionReason::ProhibitedRepoPrefix {
            repo: repository,
            prefix: prohibited_prefix,
        });
    }

    if let Some(prohibited_suffix) =
        prohibited_repo_suffixes.iter().find(|pfx| repository.ends_with(pfx.as_str()))
    {
        return Err(RejectionReason::ProhibitedRepoSuffix {
            repo: repository,
            suffix: prohibited_suffix,
        });
    }

    Ok(())
}
