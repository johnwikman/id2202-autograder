//! Common reporting interface, used for functions to report in a
//! display-agnostic format, which can then be converted to other formats down
//! the line.

use std::{
    collections::BTreeMap,
    fmt::{Display, Write},
    format,
};

use itertools::Itertools;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    config::ReportingSettings,
    db::models::{JobStatus, SubmissionJobPlain, SubmissionJobWithReport, SubmissionWithReports},
    error::Error,
    utils::utc_string,
};

pub mod structured_text;

use structured_text::StructuredParagraph;

/// Returns a markdown preformatted block <pre> containing the provided text
/// `s` as verbatim, making sure to escape parts that could otherwise be
/// interpreted as HTML.
pub fn markdown_write_preformatted(dst: &mut impl Write, s: &str) -> Result<(), Error> {
    markdown_write_preformatted_with_truncation(dst, s, None)
}

/// Returns a markdown preformatted block <pre> containing the provided text
/// `s` as verbatim, making sure to escape parts that could otherwise be
/// interpreted as HTML.
pub fn markdown_write_preformatted_with_truncation(
    dst: &mut impl Write,
    s: &str,
    truncate: Option<usize>,
) -> Result<(), Error> {
    dst.write_str("<pre>\n")?;

    fn push_escape(dst: &mut impl Write, src: &str) -> Result<(), Error> {
        for c in src.chars() {
            match c {
                '<' => dst.write_str("&lt;")?,
                '>' => dst.write_str("&gt;")?,
                '&' => dst.write_str("&amp;")?,
                _ => dst.write_char(c)?,
            }
        }
        Ok(())
    }

    let l = s.len();
    if let Some(offset) = truncate {
        let half_offset = offset.div_ceil(2);
        match if let Some(half_rev_offset) = s.len().checked_sub(half_offset) {
            (s.split_at_checked(half_offset), s.split_at_checked(half_rev_offset), offset < l)
        } else {
            (None, None, false)
        } {
            (Some(l_split), Some(r_split), true) => {
                push_escape(dst, l_split.0)?;
                dst.write_str("\n...\nTRUNCATED\n...\n")?;
                push_escape(dst, r_split.1)?;
            }
            _ => {
                push_escape(dst, s)?;
            }
        }
    } else {
        push_escape(dst, s)?;
    }

    dst.write_str("\n</pre>")?;
    Ok(())
}

/// Helper function for pushing a string `s` to the buffer `dst`, escaping the
/// contents if `escape = true`. Otherwise the string is pushed directly.
fn html_write_str(dst: &mut impl Write, s: &str, escape: bool) -> Result<(), Error> {
    if escape {
        write!(dst, "{}", v_htmlescape::escape_fmt(s))?;
    } else {
        dst.write_str(s)?;
    }
    Ok(())
}

/// Helper function for pushing a string `codeblock` to the buffer `dst` with
/// a wrapping codeblock.
///
/// TODO: Extend this to allow syntax highlighting.
fn html_write_codeblock(dst: &mut impl Write, codeblock: &str, escape: bool) -> Result<(), Error> {
    dst.write_str("<div class=\"p-2 border rounded bg-body-secondary\">")?;
    dst.write_str("<pre class=\"mb-0\"><code>")?;
    html_write_str(dst, codeblock, escape)?;
    dst.write_str("</code></pre></div>")?;
    Ok(())
}

struct HTMLFormatterStr<'a> {
    s: &'a str,
    escape: bool,
}

impl<'a> Display for HTMLFormatterStr<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        html_write_str(f, self.s, self.escape).map_err(|_| std::fmt::Error)
    }
}

fn html_formatter_str<'a>(s: &'a str, escape: bool) -> HTMLFormatterStr<'a> {
    HTMLFormatterStr { s, escape }
}

/// Common report interface.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub enum Report {
    #[serde(rename = "wrapper")]
    Wrapper(ReportWrapper),
    #[serde(rename = "invalid_tag")]
    InvalidTag(ReportInvalidTag),
    #[serde(rename = "message")]
    Message(ReportMessage),
    #[serde(rename = "tag_grading")]
    TagGrading(Box<ReportTagGrading>),
}

impl Report {
    pub fn tag_grading(internal: ReportTagGrading) -> Self {
        Self::TagGrading(Box::new(internal))
    }

    /// Renders the report as markdown, storing the result in the provided
    /// String `dst`.
    pub fn render_markdown(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> Result<(), Error> {
        match self {
            Self::Wrapper(r) => r.render_markdown(settings, dst),
            Self::InvalidTag(r) => r.render_markdown(settings, dst),
            Self::Message(r) => r.render_markdown(settings, dst),
            Self::TagGrading(r) => r.render_markdown(settings, dst),
        }
    }

    pub fn formatter_markdown<'a>(
        &'a self,
        settings: &'a ReportingSettings,
    ) -> MarkdownFormatterMetaReport<'a> {
        MarkdownFormatterMetaReport { meta_report: MetaReport::Transient(self), settings }
    }

    pub fn render_html(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
        escape: bool,
        header_level: usize,
    ) -> Result<(), Error> {
        match self {
            Self::Wrapper(r) => r.render_html(settings, dst, escape, header_level),
            Self::InvalidTag(r) => r.render_html(settings, dst, escape, header_level),
            Self::Message(r) => r.render_html(settings, dst, escape, header_level),
            Self::TagGrading(r) => r.render_html(settings, dst, escape, header_level),
        }
    }
}

/// A report assembled for display out of things that are stored separately.
/// This borrows what it renders and has no serialized form, so unlike a
/// [Report] it cannot be written to the database.
#[derive(Debug, Clone)]
pub enum MetaReport<'a> {
    /// A report that stands on its own, with no job describing it.
    Transient(&'a Report),

    /// The jobs of a single submission.
    JobResults(MetaJobResultsReport<'a>),

    /// Several reports, rendered in order and separated from one another.
    Compound(Vec<MetaReport<'a>>),

    /// Structured text that does not make sense as a report
    Structured(StructuredParagraph<'a>),
}

impl<'a> From<&'a Report> for MetaReport<'a> {
    fn from(value: &'a Report) -> Self {
        Self::Transient(value)
    }
}

impl<'a> MetaReport<'a> {
    /// Everything there is to show for a submission: anything concerning it as
    /// a whole, followed by the results of its jobs. Renders nothing if it has
    /// neither.
    pub fn of_submission(sub: &'a SubmissionWithReports) -> Self {
        let mut parts = Vec::new();
        if let Some(report) = &sub.report {
            parts.push(Self::Transient(report));
        }
        if !sub.jobs.is_empty() {
            parts.push(Self::JobResults(MetaJobResultsReport { jobs: &sub.jobs }));
        }
        Self::Compound(parts)
    }

    pub fn render_markdown(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> Result<(), Error> {
        match self {
            Self::Transient(r) => r.render_markdown(settings, dst),
            Self::JobResults(r) => r.render_markdown(settings, dst),
            Self::Compound(parts) => {
                for (i, part) in parts.iter().enumerate() {
                    if i > 0 {
                        dst.write_str("\n\n")?;
                    }
                    part.render_markdown(settings, dst)?;
                }
                Ok(())
            }
            Self::Structured(p) => p.render_markdown(settings, dst).map_err(|e| e.into()),
        }
    }

    pub fn formatter_markdown(
        &self,
        settings: &'a ReportingSettings,
    ) -> MarkdownFormatterMetaReport<'a> {
        MarkdownFormatterMetaReport { meta_report: self.clone(), settings }
    }

    pub fn render_html(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
        escape: bool,
        header_level: usize,
    ) -> Result<(), Error> {
        match self {
            Self::Transient(r) => r.render_html(settings, dst, escape, header_level),
            Self::JobResults(r) => r.render_html(settings, dst, escape, header_level),
            Self::Compound(parts) => {
                for part in parts {
                    dst.write_str("<div>")?;
                    part.render_html(settings, dst, escape, header_level)?;
                    dst.write_str("</div>")?;
                }
                Ok(())
            }
            Self::Structured(p) => p.render_html(settings, dst).map_err(|e| e.into()),
        }
    }
}

pub struct MarkdownFormatterMetaReport<'a> {
    meta_report: MetaReport<'a>,
    settings: &'a ReportingSettings,
}

impl<'a> std::fmt::Display for MarkdownFormatterMetaReport<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.meta_report.render_markdown(self.settings, f).map_err(|_| std::fmt::Error)
    }
}

/// The results of every job on one submission, each rendered together with the
/// job metadata describing how it was graded.
#[derive(Debug, Copy, Clone)]
pub struct MetaJobResultsReport<'a> {
    pub jobs: &'a [SubmissionJobWithReport],
}

impl<'a> MetaJobResultsReport<'a> {
    /// The explanatory text shown above the results.
    fn prelude(settings: &ReportingSettings) -> Vec<StructuredParagraph<'a>> {
        vec![
            StructuredParagraph::plain_str(
                "Tests are grouped together into categories. Each category contains a set of \
                 test cases that evaluate a specific aspect of your program.",
            ),
            StructuredParagraph::Itemized(vec![
                format!(
                    "The symbol {} indicates that all tests in the category passed.",
                    settings.markdown.symbol_ok
                )
                .into(),
                format!(
                    "The symbol {} indicates that not all tests were run in this category. This \
                     is usually due to a previous test timeout.",
                    settings.markdown.symbol_skipped
                )
                .into(),
                format!(
                    "The symbol {} indicates that at least one test in the category failed.",
                    settings.markdown.symbol_failed
                )
                .into(),
            ]),
            match settings.shown_failures {
                1 => StructuredParagraph::plain_str(
                    "Additionally, for the first test that fails, you will also get more \
                     detailed information after the main overview.",
                ),
                n => StructuredParagraph::plain(format!(
                    "Additionally, for the first {n} tests that fail, you will also get more \
                     detailed information after the main overview."
                )),
            },
        ]
    }

    /// Why a job holds no report, phrased for the student.
    fn without_report(job: &SubmissionJobPlain) -> String {
        match job.status {
            JobStatus::NotStarted => match job.eligible_at {
                Some(t) => format!(
                    "This tag has not been graded yet. It is rate-limited, and will be \
                     eligible for grading after {}.",
                    utc_string(&t)
                ),
                None => "This tag has not been graded yet.".to_string(),
            },
            JobStatus::Running => "This tag is currently being graded.".to_string(),
            JobStatus::Superseded => {
                "This tag was never graded, because a newer submission requested the same tag."
                    .to_string()
            }
            JobStatus::Cancelled => {
                "This tag was never graded, because the grading was cancelled by the course \
                 staff."
                    .to_string()
            }
            JobStatus::Rejected => {
                "This tag was never graded, because the grading budget for this tag has been \
                 used up."
                    .to_string()
            }
            _ => "No report was generated for this tag.".to_string(),
        }
    }

    pub fn render_markdown(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> Result<(), Error> {
        dst.write_str("# Submission Results")?;

        for block in Self::prelude(settings) {
            write!(dst, "\n\n")?;
            block.render_markdown(settings, dst)?;
        }

        for jwr in self.jobs {
            dst.write_str("\n\n")?;
            match &jwr.report {
                Some(r) => r.render_markdown(settings, dst)?,
                None => write!(
                    dst,
                    "## Results for tag `{}`\n\n_{}_",
                    jwr.job.tag,
                    Self::without_report(&jwr.job)
                )?,
            }
        }

        Ok(())
    }

    pub fn render_html(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
        escape: bool,
        header_level: usize,
    ) -> Result<(), Error> {
        /// Writes one row of a job's detail table. `code` wraps each value in a
        /// `<code>` element.
        fn detail_row(
            dst: &mut impl Write,
            escape: bool,
            label: &str,
            values: &[&str],
            code: bool,
        ) -> Result<(), Error> {
            write!(dst, "<tr><th scope=\"row\" class=\"w-25\">{label}</th><td>")?;
            for (i, v) in values.iter().enumerate() {
                if i > 0 {
                    dst.write_str(", ")?;
                }
                if code {
                    write!(dst, "<code>{}</code>", html_formatter_str(v, escape))?;
                } else {
                    html_write_str(dst, v, escape)?;
                }
            }
            dst.write_str("</td></tr>")?;
            Ok(())
        }

        let md = &settings.markdown;

        write!(dst, "<div class=\"text-center\"><h{header_level}>Results</h{header_level}></div>")?;

        for par in Self::prelude(settings) {
            par.render_html(settings, dst)?;
        }

        dst.write_str("<div class=\"accordion\">")?;
        for (i, jwr) in self.jobs.iter().enumerate() {
            let job = &jwr.job;

            // A failing tag is what the student came to read, so it is the only
            // one that opens by itself.
            let (symbol, code_class, header_class, expanded, badge_class) = if job.status
                == JobStatus::Success
            {
                (&md.symbol_ok, "text-success", "", false, None)
            } else if job.status.is_voided() {
                (
                    &md.symbol_voided,
                    "text-warning",
                    "bg-warning-subtle",
                    false,
                    Some("text-bg-warning"),
                )
            } else if !job.status.is_finished() {
                (&md.symbol_waiting, "text-body-secondary", "", false, Some("text-bg-secondary"))
            } else {
                (&md.symbol_failed, "text-danger", "bg-danger-subtle", true, None)
            };

            let item_id = format!("submissionJob{i}");
            let details_id = format!("submissionJobDetails{i}");

            dst.write_str("<div class=\"accordion-item\">")?;
            dst.write_str("<h2 class=\"accordion-header\">")?;
            write!(
                dst,
                "<button class=\"accordion-button {header_class}{}\" type=\"button\" data-bs-toggle=\"collapse\" data-bs-target=\"#{item_id}\" aria-expanded=\"{expanded}\" aria-controls=\"{item_id}\">",
                if expanded { "" } else { " collapsed" }
            )?;
            write!(
                dst,
                "<h4 class=\"my-0 flex-grow-1\">{} <code class=\"{code_class}\">{}</code></h4>",
                html_formatter_str(symbol, escape),
                html_formatter_str(&job.tag, escape)
            )?;
            if let Some(badge_class) = badge_class {
                let status = job.status.to_string();
                write!(
                    dst,
                    "<span class=\"me-3\"><span class=\"badge {badge_class}\">{}</span></span>",
                    html_formatter_str(&status, escape)
                )?;
            }
            dst.write_str("</button></h2>")?;

            write!(
                dst,
                "<div id=\"{item_id}\" class=\"accordion-collapse collapse{}\">",
                if expanded { " show" } else { "" }
            )?;
            dst.write_str("<div class=\"accordion-body\">")?;

            dst.write_str(
                "<div class=\"d-flex flex-wrap align-items-center column-gap-3 row-gap-1 mb-3\">",
            )?;
            let derived_from: Vec<&String> =
                job.requested_as.iter().filter(|r| **r != job.tag).collect();
            if !derived_from.is_empty() {
                dst.write_str("<span><em class=\"text-body-secondary\">(Derived from ")?;
                for (k, t) in derived_from.iter().enumerate() {
                    if k > 0 {
                        dst.write_str(", ")?;
                    }
                    write!(dst, "<code>{}</code>", html_formatter_str(t, escape))?;
                }
                dst.write_str(")</em></span>")?;
            }
            write!(
                dst,
                "<button class=\"btn btn-outline-secondary btn-sm py-0 px-2\" style=\"--bs-btn-font-size: .75rem;\" type=\"button\" data-bs-toggle=\"collapse\" data-bs-target=\"#{details_id}\" aria-controls=\"{details_id}\">Job details</button>"
            )?;
            dst.write_str("</div>")?;

            write!(dst, "<div class=\"collapse\" id=\"{details_id}\">")?;
            dst.write_str("<div class=\"border rounded overflow-hidden mb-3\">")?;
            dst.write_str("<table class=\"table table-striped table-hover mb-0\"><tbody>")?;

            detail_row(dst, escape, "Status", &[&job.status.to_string()], false)?;
            let requested_as: Vec<&str> = job.requested_as.iter().map(String::as_str).collect();
            detail_row(dst, escape, "Requested As", &requested_as, true)?;
            if let Some(t) = job.eligible_at {
                detail_row(dst, escape, "Eligible At", &[&utc_string(&t)], false)?;
            }
            if let Some(t) = job.started_at {
                detail_row(dst, escape, "Started At", &[&utc_string(&t)], false)?;
            }
            if let Some(t) = job.finished_at {
                detail_row(dst, escape, "Finished At", &[&utc_string(&t)], false)?;
            }
            if let (Some(from), Some(to)) = (job.started_at, job.finished_at) {
                let total = (to - from).num_seconds().max(0);
                let duration = match (
                    total / 86400,
                    (total % 86400) / 3600,
                    (total % 3600) / 60,
                    total % 60,
                ) {
                    (0, 0, 0, s) => format!("{s}s"),
                    (0, 0, m, s) => format!("{m}m {s}s"),
                    (0, h, m, s) => format!("{h}h {m}m {s}s"),
                    (d, h, m, s) => format!("{d}d {h}h {m}m {s}s"),
                };
                detail_row(dst, escape, "Duration", &[&duration], false)?;
            }
            if let Some(t) = job.voided_at {
                detail_row(dst, escape, "Voided At", &[&utc_string(&t)], false)?;
            }
            if let Some(runner) = job.assigned_runner_id {
                detail_row(dst, escape, "Assigned Runner", &[&runner.to_string()], false)?;
            }

            dst.write_str("</tbody></table></div></div>")?;

            match &jwr.report {
                Some(r) => r.render_html(settings, dst, escape, header_level + 1)?,
                None => {
                    dst.write_str("<p class=\"text-body-secondary fst-italic mb-0\">")?;
                    html_write_str(dst, &Self::without_report(job), escape)?;
                    dst.write_str("</p>")?;
                }
            }

            dst.write_str("</div></div></div>")?;
        }
        dst.write_str("</div>")?;

        Ok(())
    }
}

/// Wraps one or more reports into a single report, with the option to include
/// some surrounding metadata.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ReportWrapper {
    /// Optional title to include
    pub title: Option<String>,

    /// Wrapped reports
    pub reports: Vec<Report>,
}

impl ReportWrapper {
    /// Returns a markdown representation of the wrapper, showing the markdown
    /// for each report contained within.
    pub fn render_markdown(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> Result<(), Error> {
        if let Some(t) = &self.title {
            dst.write_str(t)?;
        }

        for (i, r) in self.reports.iter().enumerate() {
            if i > 0 || self.title.is_some() {
                dst.write_str("\n\n")?;
            }
            r.render_markdown(settings, dst)?;
        }

        Ok(())
    }

    /// Renders this wrapper report as HTML in the provided sailfish buffer.
    pub fn render_html(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
        escape: bool,
        header_level: usize,
    ) -> Result<(), Error> {
        if let Some(title) = &self.title {
            write!(dst, "<h{header_level}>")?;
            html_write_str(dst, title, escape)?;
            write!(dst, "</h{header_level}>")?;
        }

        for r in &self.reports {
            dst.write_str("<div>")?;
            r.render_html(settings, dst, escape, header_level + 1)?;
            dst.write_str("</div>")?;
        }

        Ok(())
    }
}

/// A report stating that an invalid tag has been received.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ReportInvalidTag {
    /// The tag name that was received
    pub tag_name: String,

    /// List of known grading tags
    pub known_grading_tags: Vec<String>,

    /// List of known tag groups, from which grading tags can be derived
    pub known_tag_groups: BTreeMap<String, Vec<String>>,
}

impl ReportInvalidTag {
    /// Render the invalid tag report on a GitHub markdown friendly format
    pub fn render_markdown(
        &self,
        _settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> Result<(), Error> {
        write!(dst, "Unknown tag:`{}`", self.tag_name)?;

        if !self.known_grading_tags.is_empty() {
            dst.write_str("\n\n### Known grading tags\n\n")?;
            for (i, t) in self.known_grading_tags.iter().enumerate() {
                if i > 0 {
                    dst.write_str("\n")?;
                }
                write!(dst, "* `{}`", t)?;
            }
        }

        if !self.known_tag_groups.is_empty() {
            dst.write_str("\n\n### Known tag groups\n\n")?;
            dst.write_str("| Group Name | Contained Grading Tags |\n")?;
            dst.write_str("| ---------- | ---------------------- |\n")?;
            for (g, tagnames) in &self.known_tag_groups {
                write!(dst, "| `{g}` | ")?;
                for (i, t) in tagnames.iter().enumerate() {
                    if i > 0 {
                        dst.write_str(", ")?;
                    }
                    write!(dst, "`{t}`")?;
                }

                dst.write_str(" |\n")?;
            }
            dst.write_str("\n")?; // important with double LF after table
        }

        Ok(())
    }

    /// Renders this invalid tag report as HTML in the provided sailfish buffer.
    pub fn render_html(
        &self,
        _settings: &ReportingSettings,
        dst: &mut impl Write,
        escape: bool,
        header_level: usize,
    ) -> Result<(), Error> {
        write!(
            dst,
            "<p>Received invalid grading tag: <code>{}</code></p>",
            html_formatter_str(&self.tag_name, escape)
        )?;

        if !self.known_grading_tags.is_empty() {
            write!(dst, "<h{header_level}>Known Grading Tags</h{header_level}>")?;
            write!(dst, "<ul>")?;
            for t in &self.known_grading_tags {
                write!(dst, "<li><code>{}</code></li>", html_formatter_str(t, escape))?;
            }
            write!(dst, "</ul>")?;
        }

        if !self.known_tag_groups.is_empty() {
            write!(dst, "<h{header_level}>Known Tag Groups</h{header_level}>")?;
            write!(dst, "<table class=\"table table-striped table-hover\">")?;
            write!(dst, "<thead><tr>")?;
            write!(dst, "<th scope=\"col\">Group Name</th>")?;
            write!(dst, "<th scope=\"col\">Contained Grading Tags</th>")?;
            write!(dst, "</tr></thead>")?;
            write!(dst, "<tbody>")?;
            for (groupname, contained_tags) in &self.known_tag_groups {
                write!(dst, "<tr>")?;
                write!(dst, "<td><code>{}</code></td>", html_formatter_str(groupname, escape))?;
                write!(
                    dst,
                    "<td>{}</td>",
                    contained_tags.iter().format_with(", ", |tag, f| f(&format_args!(
                        "<code>{}</code>",
                        html_formatter_str(tag, escape)
                    )))
                )?;
                write!(dst, "</tr>")?;
            }
            write!(dst, "</tbody></table>")?;
        }

        Ok(())
    }
}

/// A simple message reported as a raw string.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ReportMessage {
    /// The message to display
    pub msg: String,
}

impl ReportMessage {
    /// Simply returns the string contained within, ensuring that characters
    /// that would be formatted as markdown are escaped.
    pub fn render_markdown(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> Result<(), Error> {
        StructuredParagraph::plain_str(&self.msg)
            .render_markdown(settings, dst)
            .map_err(Error::from)
    }

    /// Renders this message as a single HTML paragraph in the provided sailfish buffer.
    pub fn render_html(
        &self,
        _settings: &ReportingSettings,
        dst: &mut impl Write,
        escape: bool,
        _header_level: usize,
    ) -> Result<(), Error> {
        dst.write_str("<p>")?;
        html_write_str(dst, &self.msg, escape)?;
        dst.write_str("</p>")?;

        Ok(())
    }
}

/// A report of grading a single tag.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ReportTagGrading {
    /// The name of the graded tag
    pub tag_name: String,

    /// The tag groups/aliases that the tag is derived from
    pub derived_from: Vec<String>,

    /// Manual indicator of whether the tag is OK. This allows for tests where
    /// it is OK to skip it, etc. The one creating the report has to indicate
    /// whether the grading is OK or not.
    pub ok: bool,

    /// Optional reason for why grading of this tag ended prematurely
    pub premature_exit_reason: Option<String>,

    /// Build report
    pub build_failure: Option<DetailsBuildFailure>,

    /// Test groups
    pub groups: Vec<DetailsTagGradingGroup>,
}

impl ReportTagGrading {
    /// Tags that this tag is derived from. Will return an empty vec if it is
    /// just itself.
    fn derivs(&self) -> Vec<&String> {
        self.derived_from.iter().filter(|d| **d != self.tag_name).collect()
    }

    /// Generate a JSON blob
    pub fn to_json(&self) -> Result<String, Error> {
        serde_json::to_string(self).map_err(|e| e.into())
    }

    /// Generate markdown, including any details that might be present.
    pub fn render_markdown(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> Result<(), Error> {
        let mut details = vec![];
        self.render_markdown_with_details(settings, dst, &mut details)?;

        for (i, detail) in details.iter().enumerate() {
            dst.write_str("\n\n")?;
            writeln!(dst, "<details id=\"detail-summary-{}\">", i + 1)?;
            writeln!(dst, "<summary>Detail {}</summary>\n", i + 1)?;
            detail.render_markdown(settings, dst)?;
            dst.write_str("\n\n</details>")?;
        }

        Ok(())
    }

    /// Generate markdown with details included separately, to be inserted later.
    fn render_markdown_with_details(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
        details: &mut Vec<DetailsTestFailure>,
    ) -> Result<(), Error> {
        write!(dst, "## Results for tag `{}`", self.tag_name)?;
        if settings.markdown.show_indicator_tag_header {
            if self.ok {
                write!(dst, " ({})", settings.markdown.symbol_ok)?;
            } else {
                write!(dst, " ({})", settings.markdown.symbol_failed)?;
            }
        }

        // Check if the tag is derived from a differently named tag group
        let derivs = self.derivs();
        if !derivs.is_empty() {
            write!(
                dst,
                "\n\n_(Derived from {})_",
                derivs.iter().format_with(", ", |s, f| f(&format_args!("`{s}`")))
            )?;
        }

        if let Some(reason) = &self.premature_exit_reason {
            write!(dst, "\n\n_({reason})_")?;
        }

        dst.write_str("\n\n")?;

        if let Some(bs) = &self.build_failure {
            bs.render_markdown(settings, dst)?;
        } else {
            let annot_tgs: Vec<_> = self.groups.iter().map(|tg| tg.annotate()).collect();

            if annot_tgs.iter().any(|atg| !atg.all_run) {
                dst.write_str("Grading process was interrupted.")?;
            } else if annot_tgs.iter().any(|atg| !atg.all_ok) {
                dst.write_str("Some test cases failed.")?;
            } else {
                write!(
                    dst,
                    "All test cases passed for this tag! {}",
                    settings.markdown.symbol_tagsuccess
                )?;
            }

            dst.write_str("\n\n")?;
            for atg in annot_tgs {
                atg.render_markdown_with_details(settings, dst, details, 0)?;
            }
        }

        Ok(())
    }

    /// Renders this tag grading report as HTML in the provided sailfish buffer.
    pub fn render_html(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
        escape: bool,
        header_level: usize,
    ) -> Result<(), Error> {
        if let Some(reason) = &self.premature_exit_reason {
            dst.write_str("<p><em>(")?;
            html_write_str(dst, reason, escape)?;
            dst.write_str(")</em></p>")?;
        }

        if let Some(bs) = &self.build_failure {
            dst.write_str("<div>")?;
            bs.render_html(settings, dst, escape, header_level + 1)?;
            dst.write_str("</div>")?;
        }

        let mut details: Vec<DetailsTestFailure> = vec![];
        let accordion_prefix = format!("detailsAccordion_{}", self.tag_name);

        if !self.groups.is_empty() {
            dst.write_str("<ul class=\"list-unstyled ms-0\">")?;
            for g in &self.groups {
                dst.write_str("<li>")?;
                g.annotate().render_html_with_details(
                    settings,
                    dst,
                    escape,
                    0,
                    &mut details,
                    &accordion_prefix,
                )?;
                dst.write_str("</li>")?;
            }
            dst.write_str("</ul>")?;
        }

        if !details.is_empty() {
            dst.write_str("<div class=\"accordion\">")?;
            for (i, detail) in details.iter().enumerate() {
                let detail_id = i + 1;
                let accordion_id = format!("{accordion_prefix}_{detail_id}");
                dst.write_str("<div class=\"accordion-item\">")?;

                dst.write_str("<h2 class=\"accordion-header\">")?;
                write!(dst, "<button class=\"accordion-button collapsed\" type=\"button\" data-bs-toggle=\"collapse\" data-bs-target=\"#{accordion_id}\" aria-expanded=\"true\" aria-controls=\"{accordion_id}\">")?;
                dst.write_str("<h5 class=\"my-0\">")?;
                write!(dst, "Detail {detail_id}")?;
                dst.write_str("</h5>")?;
                dst.write_str("</button>")?;
                dst.write_str("</h2>")?;

                write!(dst, "<div id=\"{accordion_id}\" class=\"accordion-collapse collapse\">")?;
                dst.write_str("<div class=\"accordion-body\">")?;
                detail.render_html(settings, dst, escape, header_level + 1)?;
                dst.write_str("</div></div></div>")?;
            }
            dst.write_str("</div>")?;
        }

        Ok(())
    }
}

/// Details for tag groups when creating a ReportTagGrading report.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct DetailsTagGradingGroup {
    pub group_title: String,
    pub subgroups: Vec<DetailsTagGradingGroup>,

    pub local_tests: usize,
    pub tests_run: usize,
    pub tests_passed: usize,

    pub test_details: Vec<DetailsTestFailure>,
}

/// A copy of DetailsTagGradingGroup, but used for propagating status about the
/// subsequent groups upward, for formatting purposes.
struct AnnotatedDetailsTagGradingGroup<'a> {
    pub group_title: &'a String,
    pub subgroups: Vec<AnnotatedDetailsTagGradingGroup<'a>>,

    pub local_tests: usize,
    pub tests_run: usize,
    pub tests_passed: usize,

    pub test_details: &'a Vec<DetailsTestFailure>,

    pub all_run: bool,
    pub all_ok: bool,
}

impl DetailsTagGradingGroup {
    /// Generates an annotated details group. This does a forward pass to
    /// figure out certain metadata that is needed ahead of time.
    fn annotate<'a>(&'a self) -> AnnotatedDetailsTagGradingGroup<'a> {
        // Use number of tests that have passed as an indicator
        let mut all_run = self.local_tests == self.tests_run;
        let mut all_ok = self.local_tests == self.tests_passed;

        let mut annotated_subgroups = vec![];
        for sg in &self.subgroups {
            let sg_annot = sg.annotate();
            all_run &= sg_annot.all_run;
            all_ok &= sg_annot.all_ok;
            annotated_subgroups.push(sg_annot);
        }

        AnnotatedDetailsTagGradingGroup {
            group_title: &self.group_title,
            subgroups: annotated_subgroups,
            local_tests: self.local_tests,
            tests_run: self.tests_run,
            tests_passed: self.tests_passed,
            test_details: &self.test_details,
            all_run,
            all_ok,
        }
    }
}

impl<'a> AnnotatedDetailsTagGradingGroup<'a> {
    /// Returns the status symbol to use for this grading group.
    ///
    /// Uses the symbol configured in the markdown settings.
    fn get_status_symbol<'b>(&self, settings: &'b ReportingSettings) -> &'b str {
        if !self.all_run {
            &settings.markdown.symbol_skipped
        } else if !self.all_ok {
            &settings.markdown.symbol_failed
        } else {
            &settings.markdown.symbol_ok
        }
    }
    /// Generates the test results within a grading tag.
    ///
    /// Note: the generated string will always terminate with a newline.
    fn render_markdown_with_details(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
        details: &mut Vec<DetailsTestFailure>,
        indent: usize,
    ) -> Result<(), Error> {
        write!(dst, "{:>indent$} * {} ", "", self.get_status_symbol(settings))?;

        // Bold face title if we are on top-level
        if indent == 0 {
            write!(dst, "**{}**", self.group_title)?;
        } else {
            write!(dst, "{}", self.group_title)?;
        }

        if self.local_tests > 0 {
            if self.tests_run < self.local_tests {
                write!(dst, " ({}/{} tests run)", self.tests_run, self.local_tests)?;
            } else {
                write!(dst, " ({}/{} tests passed)", self.tests_passed, self.local_tests)?;
            }
        }
        if !self.test_details.is_empty() {
            write!(dst, "\n{:>indent$}   [", "",)?;
            for (i, d) in self.test_details.iter().enumerate() {
                details.push(d.clone());
                if i > 0 {
                    dst.write_str(", ")?;
                }
                write!(
                    dst,
                    "<a href=\"#detail-summary-{}\">Detail {}</a>",
                    details.len(),
                    details.len()
                )?;
            }
            dst.write_char(']')?;
        }
        dst.write_char('\n')?;

        for sg in &self.subgroups {
            sg.render_markdown_with_details(settings, dst, details, indent + 2)?;
        }

        Ok(())
    }

    /// Renders this tag grading report as HTML in the provided sailfish buffer.
    pub fn render_html_with_details(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
        escape: bool,
        indent_level: usize,
        details: &mut Vec<DetailsTestFailure>,
        accordion_prefix: &str,
    ) -> Result<(), Error> {
        write!(dst, "<span>{}</span> ", self.get_status_symbol(settings))?;
        dst.write_str("<span>")?;
        if indent_level == 0 {
            dst.write_str("<strong>")?;
        }
        html_write_str(dst, self.group_title, escape)?;
        if indent_level == 0 {
            dst.write_str("</strong>")?;
        }
        if self.local_tests > 0 {
            if self.tests_run < self.local_tests {
                write!(dst, " ({}/{} tests run)", self.tests_run, self.local_tests)?;
            } else {
                write!(dst, " ({}/{} tests passed)", self.tests_passed, self.local_tests)?;
            }
        }
        dst.write_str("</span>")?;
        for detail in self.test_details {
            details.push(detail.clone());
            let target_id = format!("{accordion_prefix}_{}", details.len());
            dst.write_str("<button type=\"button\" class=\"btn btn-outline-primary btn-sm ms-2 mb-1 py-0 px-2\"")?;
            write!(dst, " data-bs-toggle=\"collapse\" data-bs-target=\"#{target_id}\" aria-controls=\"{target_id}\"")?;
            dst.write_str(" style=\"--bs-btn-font-size: .75rem;\">")?;
            write!(dst, "Detail {}", details.len())?;
            dst.write_str("</button>")?;
        }

        if !self.subgroups.is_empty() {
            dst.write_str("<ul class=\"list-unstyled ms-4\">")?;
            for g in &self.subgroups {
                dst.write_str("<li>")?;
                g.render_html_with_details(
                    settings,
                    dst,
                    escape,
                    indent_level + 1,
                    details,
                    accordion_prefix,
                )?;
                dst.write_str("</li>")?;
            }
            dst.write_str("</ul>")?;
        }

        Ok(())
    }
}

/// Detailed information about a failed build.
#[derive(Debug, Clone, Deserialize, Serialize, Default, JsonSchema)]
pub struct DetailsBuildFailure {
    /// A message describing the kind of build failure
    pub msg: String,

    /// The source directory to build in
    pub srcdir: Option<String>,

    /// The build command
    pub cmd: Option<String>,

    pub exit_code: Option<i32>,
    pub captured_stdout: Option<String>,
    pub captured_stderr: Option<String>,

    /// Whether the error was due to a missing source directory
    pub missing_source_directory: bool,

    /// A list of prohibited MIME-type files
    pub prohibited_mimetype_files: Vec<MIMETypeInfo>,

    /// An option additional description to be shown at the end of the detail.
    pub suffix_message: Option<String>,
}

impl DetailsBuildFailure {
    pub fn render_markdown(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> Result<(), Error> {
        write!(dst, "{} {}", settings.markdown.symbol_build, self.msg)?;

        if let Some(dir) = &self.srcdir {
            write!(dst, "\n\n**Source directory:** `{}`", dir)?;
        }
        if let Some(cmd) = &self.cmd {
            write!(dst, "\n\n**Build command:** `{}`", cmd)?;
        }
        if let Some(code) = &self.exit_code {
            write!(dst, "\n\n**Exit code:** `{}`", code)?;
        }
        if self.missing_source_directory {
            dst.write_str("\n\n**The expected source directory is missing in your submission.**")?;
        }
        if !self.prohibited_mimetype_files.is_empty() {
            dst.write_str("\n\n**Prohibited files in your solution:**\n")?;
            for mimeinfo in &self.prohibited_mimetype_files {
                dst.write_str("\n * ")?;
                mimeinfo.render_markdown(settings, dst)?;
            }
        }
        if let Some(sout) = &self.captured_stdout {
            dst.write_str("\n\n### Captured Standard Output\n\n")?;
            markdown_write_preformatted_with_truncation(
                dst,
                sout,
                Some(settings.markdown.truncate_len),
            )?;
        }
        if let Some(serr) = &self.captured_stdout {
            dst.write_str("\n\n### Captured Standard Error\n\n")?;
            markdown_write_preformatted_with_truncation(
                dst,
                serr,
                Some(settings.markdown.truncate_len),
            )?;
        }

        if let Some(msg) = &self.suffix_message {
            dst.write_str(msg)?;
        }

        Ok(())
    }

    /// Render as HTML in the provided sailfish buffer.
    pub fn render_html(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
        escape: bool,
        header_level: usize,
    ) -> Result<(), Error> {
        write!(dst, "<h{header_level}>Build Failure</h{header_level}>")?;

        dst.write_str("<p>")?;
        html_write_str(dst, &self.msg, escape)?;
        dst.write_str("</p>")?;

        if let Some(dir) = &self.srcdir {
            write!(dst, "<p><strong>")?;
            if self.missing_source_directory {
                write!(dst, "Source directory not found in submission")?;
            } else {
                write!(dst, "Source directory")?;
            }
            dst.write_str(": </strong><code>")?;
            html_write_str(dst, dir, escape)?;
            dst.write_str("</code></p>")?;
        }

        if let Some(cmd) = &self.cmd {
            dst.write_str("<p><strong>Build command: </strong><code>")?;
            html_write_str(dst, cmd, escape)?;
            dst.write_str("</code></p>")?;
        }

        if let Some(code) = &self.exit_code {
            write!(dst, "<p><strong>Exit code: </strong><code>{code}</code></p>")?;
        }

        if !self.prohibited_mimetype_files.is_empty() {
            dst.write_str("<p><strong>Prohibited files in your solution:</strong></p>")?;
            dst.write_str("<ul>")?;
            for mimeinfo in &self.prohibited_mimetype_files {
                dst.write_str("<li>")?;
                mimeinfo.render_html(settings, dst, escape, header_level + 1)?;
                dst.write_str("</li>")?;
            }
            dst.write_str("</ul>")?;
        }
        if let Some(sout) = &self.captured_stdout {
            dst.write_str("<p><strong>Captured Standard Output</strong></p>")?;
            html_write_codeblock(dst, sout, escape)?;
        }
        if let Some(serr) = &self.captured_stdout {
            dst.write_str("<p><strong>Captured Standard Error</strong></p>")?;
            html_write_codeblock(dst, serr, escape)?;
        }

        if let Some(msg) = &self.suffix_message {
            dst.write_str("<p>")?;
            html_write_str(dst, msg, escape)?;
            dst.write_str("</p>")?;
        }

        Ok(())
    }
}

/// Detailed information when a test case has failed.
#[derive(Debug, Clone, Deserialize, Serialize, Default, JsonSchema)]
pub struct DetailsTestFailure {
    /// Additional reasons to state as failure causes
    pub additional_failure_causes: Vec<String>,

    /// Optional description of the test case that failed
    pub description: Option<String>,

    /// The command that was run as part of the test case.
    pub command: Option<String>,

    /// The text provided on standard input
    pub stdin_contents: Option<SourceFileInfo>,

    /// Contents of the files provided as input to the run test.
    pub input_file_contents: Vec<SourceFileInfo>,

    /// Additional file contents to be listed, which are not provided directly
    /// as input to the run program.
    ///
    /// The list is specified as [(Title, Source), ...]
    pub additional_files: Vec<(String, SourceFileInfo)>,

    /// The return code that was captured from running the program
    pub code_captured: Option<i32>,
    /// Information about potential mismatch in the return code (if Some). If
    /// None, there was no mismatch in the return code.
    pub code_mismatch: Option<MismatchInfo<i32>>,

    /// Optionally captured standard output
    pub stdout_captured: Option<String>,
    /// Information about potential mismatch in standard output (if Some). If
    /// None, there was no mismatch in standard output.
    pub stdout_mismatch: Option<MismatchInfo<String>>,

    /// Optionally captured standard error
    pub stderr_captured: Option<String>,
    /// Information about potential mismatch in standard error (if Some). If
    /// None, there was no mismatch in standard error.
    pub stderr_mismatch: Option<MismatchInfo<String>>,

    /// Checked files as part of "check_file_exists"
    pub checked_files: Vec<String>,

    /// A list of MIME-type mismatches
    pub mimetype_mismatch_files: Vec<MIMETypeInfo>,
}

impl DetailsTestFailure {
    /// Collect the failure causes, in addition to the explicitly provided ones.
    fn summarize_fail_causes(&self) -> Vec<&str> {
        let mut fail_causes: Vec<&str> =
            self.additional_failure_causes.iter().map(|s| s.as_str()).collect();
        if self.code_mismatch.is_some() {
            fail_causes.push("Return code mismatch.");
        }
        if self.stdout_mismatch.is_some() {
            fail_causes.push("Standard output mismatch.");
        }
        if self.stderr_mismatch.is_some() {
            fail_causes.push("Standard error mismatch.");
        }
        fail_causes
    }

    fn render_markdown(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> Result<(), Error> {
        // Helper function for adding spacing between each compoenent
        let mut spacing_state = false;
        fn component_spacing(dst: &mut impl Write, spacing_state: &mut bool) -> Result<(), Error> {
            if *spacing_state {
                dst.write_str("\n\n")?;
            }
            *spacing_state = true;
            Ok::<_, Error>(())
        }

        let fail_causes = self.summarize_fail_causes();
        if !fail_causes.is_empty() {
            component_spacing(dst, &mut spacing_state)?;
            dst.write_str("**Test failed for the following reasons:**\n")?;
            for s in &fail_causes {
                write!(dst, "\n * {}", s)?;
            }
        }

        if let Some(desc) = &self.description {
            component_spacing(dst, &mut spacing_state)?;
            StructuredParagraph::plain_str(desc)
                .render_markdown(settings, dst)
                .map_err(Error::from)?;
        }

        if !self.checked_files.is_empty() {
            component_spacing(dst, &mut spacing_state)?;
            dst.write_str("**The following files were checked for in solution:**\n")?;
            for check_file in &self.checked_files {
                write!(dst, "\n * `{}`", check_file)?;
            }
        }

        if !self.mimetype_mismatch_files.is_empty() {
            component_spacing(dst, &mut spacing_state)?;
            dst.write_str("**MIME-type mismatches in your solution:**\n")?;
            for mimeinfo in &self.mimetype_mismatch_files {
                write!(dst, "\n * ")?;
                mimeinfo.render_markdown(settings, dst)?;
            }
        }

        if let Some(cmd) = &self.command {
            component_spacing(dst, &mut spacing_state)?;
            write!(dst, "**Command:** `{}`", cmd)?;
        }

        if let Some(code) = &self.code_captured {
            component_spacing(dst, &mut spacing_state)?;
            write!(dst, "**Return code:** `{}`", code)?;
        }

        if let Some(stdin) = &self.stdin_contents {
            component_spacing(dst, &mut spacing_state)?;
            dst.write_str("### Standard Input\n\n")?;
            stdin.render_markdown(settings, dst)?;
        }

        for (i, infile) in self.input_file_contents.iter().enumerate() {
            component_spacing(dst, &mut spacing_state)?;
            dst.write_str("### Input File")?;
            if self.input_file_contents.len() > 1 {
                write!(dst, " {}", i + 1)?;
            }
            dst.write_str("\n\n")?;
            infile.render_markdown(settings, dst)?;
        }

        if let Some(mm) = &self.code_mismatch {
            component_spacing(dst, &mut spacing_state)?;
            mm.render_markdown(settings, dst, "Return Code Mismatch", "code")?;
        }

        if let Some(mm) = &self.stdout_mismatch {
            component_spacing(dst, &mut spacing_state)?;
            mm.render_markdown(settings, dst, "Standard Output Mismatch", "stdout")?;
        }
        if let Some(mm) = &self.stderr_mismatch {
            component_spacing(dst, &mut spacing_state)?;
            mm.render_markdown(settings, dst, "Standard Error Mismatch", "stderr")?;
        }

        for (title, file_info) in &self.additional_files {
            component_spacing(dst, &mut spacing_state)?;
            write!(dst, "### {}\n\n", title)?;
            file_info.render_markdown(settings, dst)?;
        }

        if let Some(cap) = &self.stdout_captured {
            component_spacing(dst, &mut spacing_state)?;
            dst.write_str("### Captured Standard Output\n\n")?;
            markdown_write_preformatted_with_truncation(
                dst,
                cap,
                Some(settings.markdown.truncate_len),
            )?;
        }

        if let Some(cap) = &self.stderr_captured {
            component_spacing(dst, &mut spacing_state)?;
            dst.write_str("### Captured Standard Error\n\n")?;
            markdown_write_preformatted_with_truncation(
                dst,
                cap,
                Some(settings.markdown.truncate_len),
            )?;
        }

        Ok(())
    }

    /// Renders this as HTML in the provided sailfish buffer.
    fn render_html(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
        escape: bool,
        header_level: usize,
    ) -> Result<(), Error> {
        let fail_causes = self.summarize_fail_causes();
        if !fail_causes.is_empty() {
            dst.write_str("<p><strong>Test failed for the following reasons:</strong></p>")?;
            dst.write_str("<ul>")?;
            for cause in &fail_causes {
                dst.write_str("<li>")?;
                html_write_str(dst, cause, escape)?;
                dst.write_str("</li>")?;
            }
            dst.write_str("</ul>")?;
        }

        if let Some(desc) = &self.description {
            dst.write_str("<p>")?;
            html_write_str(dst, desc, escape)?;
            dst.write_str("</p>")?;
        }

        if !self.checked_files.is_empty() {
            dst.write_str(
                "<p><strong>The following files were checked for in solution:</strong></p>",
            )?;
            dst.write_str("<ul>")?;
            for check_file in &self.checked_files {
                dst.write_str("<li><code>")?;
                html_write_str(dst, check_file, escape)?;
                dst.write_str("</code></li>")?;
            }
            dst.write_str("</ul>")?;
        }

        if !self.mimetype_mismatch_files.is_empty() {
            dst.write_str("<p><strong>MIME-type mismatches in your solution:</strong></p>")?;
            dst.write_str("<ul>")?;
            for mimeinfo in &self.mimetype_mismatch_files {
                dst.write_str("<li>")?;
                mimeinfo.render_html(settings, dst, escape, header_level + 1)?;
                dst.write_str("</li>")?;
            }
            dst.write_str("</ul>")?;
        }

        if let Some(cmd) = &self.command {
            dst.write_str("<p><span><strong>Command:</strong> <code>")?;
            html_write_str(dst, cmd, escape)?;
            dst.write_str("</code></span></p>")?;
        }

        if let Some(code) = &self.code_captured {
            write!(dst, "<p><span><strong>Return code:</strong> <code>{code}</code></span></p>")?;
        }

        if let Some(stdin) = &self.stdin_contents {
            dst.write_str("<h6>Standard Input</h6>")?;
            stdin.render_html(settings, dst, escape, header_level + 1)?;
        }

        if let Some(mm) = &self.code_mismatch {
            mm.render_html(settings, dst, escape, header_level + 1, "Return Code Mismatch")?;
        }

        if let Some(mm) = &self.stdout_mismatch {
            mm.render_html(settings, dst, escape, header_level + 1, "Standard Output Mismatch")?;
        }

        if let Some(mm) = &self.stderr_mismatch {
            mm.render_html(settings, dst, escape, header_level + 1, "Standard Error Mismatch")?;
        }

        for (title, file_info) in &self.additional_files {
            dst.write_str("<h6>")?;
            html_write_str(dst, title, escape)?;
            dst.write_str("</h6>")?;
            file_info.render_html(settings, dst, escape, header_level + 1)?;
        }

        if let Some(cap) = &self.stdout_captured {
            dst.write_str("<h6 class=\"mt-3\">Captured Standard Output</h6>")?;
            html_write_codeblock(dst, cap, escape)?;
        }

        if let Some(cap) = &self.stderr_captured {
            dst.write_str("<h6 class=\"mt-3\">Captured Standard Error</h6>")?;
            html_write_codeblock(dst, cap, escape)?;
        }
        Ok(())
    }
}

/// Information about a mismatch when comparing what was received to the
/// allowed alternatives.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MismatchInfo<A> {
    /// The output that was received from the program
    pub received: A,
    /// The allowed alternatives that this could be set to
    pub allowed_alternatives: Vec<A>,
    /// Optional additional messages
    pub msgs: Vec<String>,
}

impl<A> MismatchInfo<A> {
    fn render_markdown_header(&self, dst: &mut impl Write, title: &str) -> Result<(), Error> {
        write!(dst, "### {}\n\n", title)?;

        for msg in &self.msgs {
            write!(dst, "{}\n\n", msg)?;
        }

        Ok(())
    }

    fn render_html_begin(
        &self,
        dst: &mut impl Write,
        escape: bool,
        title: &str,
    ) -> Result<(), Error> {
        dst.write_str("<div class=\"card mt-3 border border-2 border-danger rounded\">")?;
        write!(
            dst,
            "<h4 class=\"card-header border-danger bg-danger-subtle\">{}</h4>",
            html_formatter_str(title, escape)
        )?;
        dst.write_str("<div class=\"card-body\">")?;
        for msg in &self.msgs {
            write!(dst, "<p><mark>{}</mark></p>", html_formatter_str(msg, escape))?;
        }

        Ok(())
    }

    fn render_html_end(&self, dst: &mut impl Write) -> Result<(), Error> {
        dst.write_str("</div></div>")?;

        Ok(())
    }
}

impl MismatchInfo<String> {
    /// For strings, we assume that each string corresponds to a code block,
    /// and will be presented in verbatim.
    fn render_markdown(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
        title: &str,
        output_name: &str,
    ) -> Result<(), Error> {
        self.render_markdown_header(dst, title)?;

        write!(dst, "**Received {}**:\n\n", output_name)?;

        markdown_write_preformatted_with_truncation(
            dst,
            &self.received,
            Some(settings.markdown.truncate_len),
        )?;

        if let &[alt] = &self.allowed_alternatives.as_slice() {
            write!(dst, "\n\n**Expected {}**:\n\n", output_name)?;

            markdown_write_preformatted_with_truncation(
                dst,
                alt,
                Some(settings.markdown.truncate_len),
            )?;
        } else {
            dst.write_str("**Expected one of**:\n\n")?;
            for (i, alt) in self.allowed_alternatives.iter().enumerate() {
                if i > 0 {
                    dst.write_str("\n\n**or**\n\n")?;
                }
                markdown_write_preformatted_with_truncation(
                    dst,
                    alt,
                    Some(settings.markdown.truncate_len),
                )?;
            }
        }

        Ok(())
    }

    fn render_html(
        &self,
        _settings: &ReportingSettings,
        dst: &mut impl Write,
        escape: bool,
        _header_level: usize,
        title: &str,
    ) -> Result<(), Error> {
        self.render_html_begin(dst, escape, title)?;

        dst.write_str("<h6>Received</h6>")?;
        html_write_codeblock(dst, &self.received, escape)?;
        dst.write_str("<h6 class=\"mt-2\">Expected")?;
        if self.allowed_alternatives.len() > 1 {
            dst.write_str(" one of")?;
        }
        dst.write_str("</h6>")?;
        for (i, expected) in self.allowed_alternatives.iter().enumerate() {
            if i > 0 {
                dst.write_str("<span><strong>or</strong></span>")?;
            }
            html_write_codeblock(dst, expected, escape)?;
        }

        self.render_html_end(dst)
    }
}

impl MismatchInfo<i32> {
    /// For strings, we assume that each string corresponds to a code block,
    /// and will be presented in verbatim.
    fn render_markdown(
        &self,
        _settings: &ReportingSettings,
        dst: &mut impl Write,
        title: &str,
        _output_name: &str,
    ) -> Result<(), Error> {
        self.render_markdown_header(dst, title)?;

        write!(dst, "Received return code `{}`. Expected ", self.received)?;
        match self.allowed_alternatives.as_slice() {
            &[expected] => {
                write!(dst, "`{expected}`.")?;
            }
            many_expected => {
                write!(
                    dst,
                    "one of {}.",
                    many_expected.iter().format_with(", ", |ex, f| f(&format_args!("`{ex}`")))
                )?;
            }
        }

        Ok(())
    }

    fn render_html(
        &self,
        _settings: &ReportingSettings,
        dst: &mut impl Write,
        escape: bool,
        _header_level: usize,
        title: &str,
    ) -> Result<(), Error> {
        self.render_html_begin(dst, escape, title)?;

        write!(dst, "<span>Received return code <code>{}</code>. Expected ", self.received)?;
        match self.allowed_alternatives.as_slice() {
            &[expected] => {
                write!(dst, "<code>{expected}</code>.")?;
            }
            many_expected => {
                write!(
                    dst,
                    "one of {}.",
                    many_expected
                        .iter()
                        .format_with(", ", |ex, f| f(&format_args!("<code>{ex}</code>")))
                )?;
            }
        }
        dst.write_str("</span>")?;

        self.render_html_end(dst)
    }
}

/// Information about a source file to be displayed
#[derive(Debug, Clone, Deserialize, Serialize, Default, JsonSchema)]
pub struct SourceFileInfo {
    /// The contents of the file. This must always be present.
    pub content: String,

    /// Optional file extension without the dot separator. E.g. `cpp`, `java`,
    /// `py`, etc.
    pub extension: Option<String>,
}

impl SourceFileInfo {
    /// Generates a markdown representation of the source file information.
    fn render_markdown(
        &self,
        _settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> Result<(), Error> {
        dst.write_str("```")?;
        if let Some(ex) = &self.extension {
            dst.write_str(ex)?;
        }
        dst.write_str("\n")?;
        dst.write_str(&self.content)?;
        dst.write_str("\n```")?;
        Ok(())
    }

    /// Renders a code block in HTML for this source file.
    ///
    /// TODO: Add syntax highlighting. Just now it only renders a basic block.
    fn render_html(
        &self,
        _settings: &ReportingSettings,
        dst: &mut impl Write,
        escape: bool,
        _header_level: usize,
    ) -> Result<(), Error> {
        html_write_codeblock(dst, &self.content, escape)
    }
}

/// Information about a MIME-type check
#[derive(Debug, Clone, Deserialize, Serialize, Default, JsonSchema)]
pub struct MIMETypeInfo {
    /// The path that was checked
    pub path: String,

    /// The identified MIME-type
    pub mime_identified: String,

    /// The expected MIME-type
    pub mime_expected: Option<String>,
}

impl MIMETypeInfo {
    /// Generates a single-line Markdown representation of the MIME-type information.
    fn render_markdown(
        &self,
        _settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> Result<(), Error> {
        write!(dst, "`{}` (Identified as MIME-type `{}`", self.path, self.mime_identified)?;
        if let Some(expected) = &self.mime_expected {
            write!(dst, ", `{}`", expected)?;
        }
        dst.write_char(')')?;
        Ok(())
    }

    /// Renders a code block in HTML for this source file.
    ///
    /// TODO: Add syntax highlighting. Just now it only renders a basic block.
    fn render_html(
        &self,
        _settings: &ReportingSettings,
        dst: &mut impl Write,
        escape: bool,
        _header_level: usize,
    ) -> Result<(), Error> {
        dst.write_str("<code>")?;
        html_write_str(dst, &self.path, escape)?;
        dst.write_str("</code> (Identified as MIME-type <code>")?;
        html_write_str(dst, &self.mime_identified, escape)?;
        if let Some(expected) = &self.mime_expected {
            dst.write_str(", expected <code>")?;
            html_write_str(dst, expected, escape)?;
            dst.write_str("</code>")?;
        }
        dst.write_str("</code>)")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asserting::prelude::*;

    #[test]
    fn test_md_preformatted() {
        let mut dst = String::new();
        assert_that!(markdown_write_preformatted(&mut dst, "foo")).is_ok();
        assert_that!(dst).is_equal_to("<pre>\nfoo\n</pre>");

        let mut dst = String::new();
        assert_that!(markdown_write_preformatted(&mut dst, "int foo() {return 1 < 2;}")).is_ok();
        assert_that!(dst).is_equal_to("<pre>\nint foo() {return 1 &lt; 2;}\n</pre>");

        let mut dst = String::new();
        assert_that!(markdown_write_preformatted(
            &mut dst,
            "bool bar(int x) {\n  return x < 2 && x >= 2;\n}"
        ))
        .is_ok();
        assert_that!(dst).is_equal_to(
            "<pre>\nbool bar(int x) {\n  return x &lt; 2 &amp;&amp; x &gt;= 2;\n}\n</pre>",
        );
    }

    #[test]
    fn test_md_preformatted_truncated() {
        let mut dst = String::new();
        assert_that!(markdown_write_preformatted_with_truncation(&mut dst, "foo", Some(3))).is_ok();
        assert_that!(dst).is_equal_to("<pre>\nfoo\n</pre>");

        let mut dst = String::new();
        assert_that!(markdown_write_preformatted_with_truncation(
            &mut dst,
            "int foo() {return 1 < 2;}",
            Some(400)
        ))
        .is_ok();
        assert_that!(dst).is_equal_to("<pre>\nint foo() {return 1 &lt; 2;}\n</pre>");

        let mut dst = String::new();
        assert_that!(markdown_write_preformatted_with_truncation(
            &mut dst,
            "bool bar(int x) {\n  return x < 2 && x >= 2;\n}",
            Some(400)
        ))
        .is_ok();
        assert_that!(dst).is_equal_to(
            "<pre>\nbool bar(int x) {\n  return x &lt; 2 &amp;&amp; x &gt;= 2;\n}\n</pre>",
        );

        // Actual splits
        let mut dst = String::new();
        assert_that!(markdown_write_preformatted_with_truncation(&mut dst, "foo", Some(2))).is_ok();
        assert_that!(dst).is_equal_to("<pre>\nf\n...\nTRUNCATED\n...\no\n</pre>");

        let mut dst = String::new();
        assert_that!(markdown_write_preformatted_with_truncation(
            &mut dst,
            "int foo() {return 1 < 2;}",
            Some(12)
        ))
        .is_ok();
        assert_that!(dst).is_equal_to("<pre>\nint fo\n...\nTRUNCATED\n...\n &lt; 2;}\n</pre>");
    }

    #[test]
    fn test_invalid_tag_from_json() {
        let ok_blobs = [
            json::object! {
                tag_name: "foo",
                known_grading_tags: ["bar", "babar"],
                known_tag_groups: {"bara-babar": ["babar"]},
            },
            json::array!["foo", ["bar", "babar"], {"bara-babar": ["babar"]}],
        ];

        for blob in ok_blobs {
            let r: Result<ReportInvalidTag, serde_json::Error> =
                serde_json::from_str(&blob.to_string());
            assert_that!(&r).is_ok();
            let t = r.unwrap();

            assert_eq!(t.tag_name, "foo");
            assert_eq!(t.known_grading_tags, ["bar", "babar"]);
            assert_eq!(t.known_tag_groups.len(), 1);
            assert_eq!(t.known_tag_groups.get("bara-babar"), Some(&vec!["babar".to_string()]));
        }

        let bad_blobs = [
            json::object! {
                tag_name: "foo",
                known_grading_tags: ["bar", "babar"],
            },
            json::array![["bar", "babar"], "foo", {"bara-babar": ["babar"]}],
        ];

        for blob in bad_blobs {
            let bad_r: Result<ReportInvalidTag, serde_json::Error> =
                serde_json::from_str(&blob.to_string());
            assert_that!(bad_r).is_err();
        }
    }
}
