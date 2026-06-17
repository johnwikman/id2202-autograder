use std::{collections::BTreeSet, fmt::Display};

use id2202_autograder::{
    config::tag_match,
    config::Settings,
    reporting::{Report, ReportMessage, ReportWrapper},
};
use itertools::Itertools;

/// Extracts grading tags from the the string in `from`. Returns a vector (that
/// may be empty) containing all grading tags on success. En error report is
/// returned on failure.
pub fn extract_grading_tags<'a>(
    settings: &Settings,
    from: &'a str,
) -> Result<Vec<&'a str>, Report> {
    // Check for grading tags. First adding them to BTreeSet to remove any
    // duplicates unique, then converting the set back to a vector.
    let mut grading_tag_set: BTreeSet<&str> = BTreeSet::new();
    let mut s: &'a str = from;
    while s.len() > 0 {
        // We split at i + 1 because we are interested in the string that
        // follows the tag symbol.
        if let Some((_, s_after)) = s
            .find(|c: char| c == '#' || c == '%')
            .and_then(|i| s.split_at_checked(i + 1))
        {
            let (s_tag, s_rest) = tag_match(s_after);
            grading_tag_set.insert(s_tag);
            s = s_rest;
        } else {
            break;
        }
    }

    let grading_tags: Vec<&'a str> = grading_tag_set
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();

    let tag_length = grading_tags
        .iter()
        .map(|s| s.len())
        .reduce(|acc, e| acc + e)
        .unwrap_or(0usize);

    if tag_length >= settings.submission.max_tag_length {
        Err(Report::Wrapper(ReportWrapper {
            title: Some("Submission Error".to_string()),
            reports: vec![
                Report::Message(ReportMessage { msg: format!(
                    "The provided grading tags {} exceed the limit of {} characters. Your submission will not be graded.",
                    grading_tags.iter().format_with(", ", |t, f| f(&format_args!("`{t}`"))),
                    settings.submission.max_tag_length,
                )})
            ]
        }))
    } else {
        Ok(grading_tags)
    }
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
    if allowed_groups.len() > 0 {
        if !allowed_groups.iter().any(|org| org == group) {
            return Err(RejectionReason::InvalidGroup { group: group });
        }
    }

    if allowed_repo_prefixes.len() > 0 {
        let allowed_prefix = allowed_repo_prefixes
            .iter()
            .any(|pfx| repository.starts_with(pfx));
        if !allowed_prefix {
            return Err(RejectionReason::InvalidRepoPrefix { repo: repository });
        }
    }
    if allowed_repo_suffixes.len() > 0 {
        let allowed_suffix = allowed_repo_suffixes
            .iter()
            .any(|sfx| repository.ends_with(sfx));
        if !allowed_suffix {
            return Err(RejectionReason::InvalidRepoSuffix { repo: repository });
        }
    }

    if let Some(prohibited_prefix) = prohibited_repo_prefixes
        .iter()
        .find(|pfx| repository.starts_with(pfx.as_str()))
    {
        return Err(RejectionReason::ProhibitedRepoPrefix {
            repo: repository,
            prefix: prohibited_prefix,
        });
    }

    if let Some(prohibited_suffix) = prohibited_repo_suffixes
        .iter()
        .find(|pfx| repository.starts_with(pfx.as_str()))
    {
        return Err(RejectionReason::ProhibitedRepoSuffix {
            repo: repository,
            suffix: prohibited_suffix,
        });
    }

    Ok(())
}
