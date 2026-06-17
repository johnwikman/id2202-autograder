/// Common reporting interface, used for functions to report in a
/// display-agnostic format, which can then be converted to other formats down
/// the line.
use std::{
    collections::BTreeMap,
    fmt::{Display, Write},
};

use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::{config::ReportingSettings, error::Error};

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
            (
                s.split_at_checked(half_offset),
                s.split_at_checked(half_rev_offset),
                offset < l,
            )
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

/// Escapes markdown characters within the string `s`.
///
/// This escapes the following characters by putting a backslash `\` in front of them:
///
/// ```txt
/// \ ` * _ { } [ ] ( ) # + - . !
/// ```
///
/// See https://www.markdownlang.com/basic/escaping.html
pub fn markdown_write_escaped(dst: &mut impl Write, s: &str) -> Result<(), Error> {
    const ESC_CHARS: &'static str = "\\`*_{}[]()#+-.!";
    for ch in s.chars() {
        if ESC_CHARS.contains(ch) {
            dst.write_char('\\')?;
        }
        dst.write_char(ch)?;
    }
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
#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum Report {
    Wrapper(ReportWrapper),
    InvalidTag(ReportInvalidTag),
    Message(ReportMessage),
    Submission(ReportSubmission),
    TagGrading(ReportTagGrading),
}

impl Report {
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
            Self::Submission(r) => r.render_markdown(settings, dst),
            Self::TagGrading(r) => r.render_markdown(settings, dst),
        }
    }

    pub fn formatter_markdown<'a>(
        &'a self,
        settings: &'a ReportingSettings,
    ) -> MarkdownFormatterReport<'a> {
        MarkdownFormatterReport {
            report: self,
            settings: settings,
        }
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
            Self::Submission(r) => r.render_html(settings, dst, escape, header_level),
            Self::TagGrading(r) => r.render_html(settings, dst, escape, header_level),
        }
    }
}

pub struct MarkdownFormatterReport<'a> {
    report: &'a Report,
    settings: &'a ReportingSettings,
}

impl<'a> std::fmt::Display for MarkdownFormatterReport<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.report
            .render_markdown(self.settings, f)
            .map_err(|_| std::fmt::Error)
    }
}

/// Wraps one or more reports into a single report, with the option to include
/// some surrounding metadata.
#[derive(Debug, Clone, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
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

        if self.known_grading_tags.len() > 0 {
            dst.write_str("\n\n### Known grading tags\n\n")?;
            for (i, t) in self.known_grading_tags.iter().enumerate() {
                if i > 0 {
                    dst.write_str("\n")?;
                }
                write!(dst, "* `{}`", t)?;
            }
        }

        if self.known_tag_groups.len() > 0 {
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

        if self.known_grading_tags.len() > 0 {
            write!(dst, "<h{header_level}>Known Grading Tags</h{header_level}>")?;
            write!(dst, "<ul>")?;
            for t in &self.known_grading_tags {
                write!(
                    dst,
                    "<li><code>{}</code></li>",
                    html_formatter_str(t, escape)
                )?;
            }
            write!(dst, "</ul>")?;
        }

        if self.known_tag_groups.len() > 0 {
            write!(dst, "<h{header_level}>Known Tag Groups</h{header_level}>")?;
            write!(dst, "<table class=\"table table-striped table-hover\">")?;
            write!(dst, "<thead><tr>")?;
            write!(dst, "<th scope=\"col\">Group Name</th>")?;
            write!(dst, "<th scope=\"col\">Contained Grading Tags</th>")?;
            write!(dst, "</tr></thead>")?;
            write!(dst, "<tbody>")?;
            for (groupname, contained_tags) in &self.known_tag_groups {
                write!(dst, "<tr>")?;
                write!(
                    dst,
                    "<td><code>{}</code></td>",
                    html_formatter_str(groupname, escape)
                )?;
                write!(
                    dst,
                    "<td>{}</td>",
                    contained_tags
                        .iter()
                        .format_with(", ", |tag, f| f(&format_args!(
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
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReportMessage {
    /// The message to display
    pub msg: String,
}

impl ReportMessage {
    /// Simply returns the string contained within, ensuring that characters
    /// that would be formatted as markdown are escaped.
    pub fn render_markdown(
        &self,
        _settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> Result<(), Error> {
        markdown_write_escaped(dst, &self.msg)?;
        Ok(())
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

/// A report of a complete submission, constituting of variable number of grading tags.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReportSubmission {
    /// Optional reason for why a grading process ended prematurely
    pub premature_exit_reason: Option<String>,

    /// Maximum number of shown details on failure
    pub max_shown_details: Option<usize>,

    /// Reports for each individual tag
    pub tag_reports: Vec<ReportTagGrading>,
}

impl ReportSubmission {
    /// Generate markdown, with details from every subreport being included at
    /// the end.
    pub fn render_markdown(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> Result<(), Error> {
        dst.write_str("# Submission Results")?;

        if settings.markdown.show_indicator_submission_header {
            dst.write_str("( ")?;
            if self.tag_reports.iter().all(|tr| tr.ok) {
                dst.write_str(&settings.markdown.symbol_ok)?;
            } else {
                dst.write_str(&settings.markdown.symbol_failed)?;
            }
        }

        if let Some(reason) = &self.premature_exit_reason {
            write!(dst, "\n\n_({reason})_")?;
        }

        // Add the information text
        dst.write_str("\n\nTests are grouped together into categories.")?;
        dst.write_str(" Each category contains a set of test cases that evaluate a specific aspect of your program.")?;

        dst.write_str("\n\n * The symbol ")?;
        dst.write_str(&settings.markdown.symbol_ok)?;
        dst.write_str(" indicates that all tests in the category passed.")?;

        dst.write_str("\n\n * The symbol ")?;
        dst.write_str(&settings.markdown.symbol_skipped)?;
        dst.write_str(" indicates that not all tests were run in this category.")?;
        dst.write_str(" This is usually due to a previous test timeout.")?;

        dst.write_str("\n\n * The symbol ")?;
        dst.write_str(&settings.markdown.symbol_failed)?;
        dst.write_str(" indicates that at least one test in the category failed.")?;

        if let Some(max_details) = self.max_shown_details {
            dst.write_str("\n\nAdditionally, for the first ")?;
            if max_details == 1 {
                dst.write_str("test that fail")?;
            } else {
                write!(dst, "{} tests that fail", max_details)?;
            }
            dst.write_str(
                ", you will also get more detailed information after the main overview.",
            )?;
        }

        let mut details = vec![];
        for tr in &self.tag_reports {
            dst.write_str("\n\n")?;
            tr.render_markdown_with_details(settings, dst, &mut details)?;
        }

        for (i, detail) in details.iter().enumerate() {
            dst.write_str("\n\n")?;
            write!(dst, "<details id=\"detail-summary-{}\">\n", i + 1)?;
            write!(dst, "<summary>Detail {}</summary>\n\n", i + 1)?;
            detail.render_markdown(settings, dst)?;
            dst.write_str("\n\n</details>")?;
        }

        Ok(())
    }

    /// Renders this submission report as HTML in the provided sailfish buffer.
    pub fn render_html(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
        escape: bool,
        header_level: usize,
    ) -> Result<(), Error> {
        if let Some(reason) = &self.premature_exit_reason {
            dst.write_str("<p><em>")?;
            html_write_str(dst, reason, escape)?;
            dst.write_str("<em></p>")?;
        }

        dst.write_str("<div class=\"text-center\">")?;
        write!(dst, "<h{header_level}>Results</{header_level}>")?;
        dst.write_str("</div>")?;

        dst.write_str("<div class=\"accordion\">")?;
        for (i, grading_report) in self.tag_reports.iter().enumerate() {
            let accordion_id = format!("gradingTagAccordion{i}");
            let (button_bg_class, accordion_border_class, tag_code_class) = if grading_report.ok {
                //("bg-success-subtle", "border-success-subtle", "text-success")
                ("", "", "text-success")
            } else {
                //("bg-danger-subtle", "border-danger-subtle", "text-danger")
                ("bg-danger-subtle", "", "text-danger")
            };
            // Only show results if something failed
            let (show_class, collapse_class) = if grading_report.ok {
                ("", "collapsed")
            } else {
                ("show", "")
            };
            write!(
                dst,
                "<div class=\"accordion-item {accordion_border_class}\">"
            )?;
            dst.write_str("<h2 class=\"accordion-header\">")?;
            write!(dst, "<button class=\"accordion-button {button_bg_class} {collapse_class}\" type=\"button\" data-bs-toggle=\"collapse\" data-bs-target=\"#{accordion_id}\" aria-expanded=\"true\" aria-controls=\"{accordion_id}\">")?;
            dst.write_str("<h4 class=\"my-0\">")?;
            if grading_report.ok {
                dst.write_str(&settings.markdown.symbol_ok)?;
            } else {
                dst.write_str(&settings.markdown.symbol_failed)?;
            };
            write!(
                dst,
                " <code class=\"{tag_code_class}\">{}</code></h4>",
                html_formatter_str(&grading_report.tag_name, escape)
            )?;

            dst.write_str("</button>")?;
            dst.write_str("</h2>")?;
            write!(
                dst,
                "<div id=\"{accordion_id}\" class=\"accordion-collapse collapse {show_class}\">"
            )?;
            dst.write_str("<div class=\"accordion-body\">")?;
            grading_report.render_html(settings, dst, escape, header_level + 1)?;
            dst.write_str("</div></div></div>")?;
        }
        dst.write_str("</div>")?;

        Ok(())
    }
}

/// A report of grading a single tag.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReportTagGrading {
    /// The name of the graded tag
    pub tag_name: String,

    /// The tag groups/aliases that the tag is derived from
    pub derived_from: Vec<String>,

    /// Manual indicator of whether the tag is OK. This allows for tests where
    /// it is OK to skip it, etc. The one creating the report has to indicate
    /// whether the grading is OK or not.
    pub ok: bool,

    /// Build report
    pub build_failure: Option<DetailsBuildFailure>,

    /// Test groups
    pub groups: Vec<DetailsTagGradingGroup>,
}

impl ReportTagGrading {
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
            write!(dst, "<details id=\"detail-summary-{}\">\n", i + 1)?;
            write!(dst, "<summary>Detail {}</summary>\n\n", i + 1)?;
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
        write!(dst, "## Results for tag `{}`", &self.tag_name)?;
        if settings.markdown.show_indicator_tag_header {
            if self.ok {
                write!(dst, " ({})", settings.markdown.symbol_ok)?;
            } else {
                write!(dst, " ({})", settings.markdown.symbol_failed)?;
            }
        }

        // Check if the tag is derived from a differently named tag group
        let derivs: Vec<String> = self
            .derived_from
            .iter()
            .filter(|d| **d != self.tag_name)
            .map(String::to_owned)
            .collect();
        if derivs.len() > 0 {
            write!(
                dst,
                "\n\n_(Derived from {})_",
                derivs
                    .iter()
                    .format_with(", ", |s, f| f(&format_args!("`{s}`")))
            )?;
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
            if details.len() > 0 {
                dst.write_str(" See details below.")?;
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
        if self.derived_from.len() > 0 {
            dst.write_str("<p><em>(Derived from ")?;
            for (i, t) in self.derived_from.iter().enumerate() {
                if i > 0 {
                    dst.write_str(", ")?;
                }
                dst.write_str("<code>")?;
                html_write_str(dst, t, escape)?;
                dst.write_str("</code>")?;
            }
            dst.write_str(")</em></p>")?;
        }

        if let Some(bs) = &self.build_failure {
            dst.write_str("<div>")?;
            bs.render_html(settings, dst, escape, header_level + 1)?;
            dst.write_str("</div>")?;
        }

        let mut details: Vec<DetailsTestFailure> = vec![];
        let accordion_prefix = format!("detailsAccordion_{}", self.tag_name);

        if self.groups.len() > 0 {
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

        if details.len() > 0 {
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

                write!(
                    dst,
                    "<div id=\"{accordion_id}\" class=\"accordion-collapse collapse\">"
                )?;
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
#[derive(Debug, Clone, Deserialize, Serialize)]
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
            all_run: all_run,
            all_ok: all_ok,
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
        write!(
            dst,
            "{:>indent$} * {} ",
            "",
            self.get_status_symbol(settings)
        )?;

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
                write!(
                    dst,
                    " ({}/{} tests passed)",
                    self.tests_passed, self.local_tests
                )?;
            }
        }
        if self.test_details.len() > 0 {
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
        html_write_str(dst, &self.group_title, escape)?;
        if indent_level == 0 {
            dst.write_str("</strong>")?;
        }
        if self.local_tests > 0 {
            if self.tests_run < self.local_tests {
                write!(dst, " ({}/{} tests run)", self.tests_run, self.local_tests)?;
            } else {
                write!(
                    dst,
                    " ({}/{} tests passed)",
                    self.tests_passed, self.local_tests
                )?;
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

        if self.subgroups.len() > 0 {
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
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
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
        if self.prohibited_mimetype_files.len() > 0 {
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
            write!(
                dst,
                "<p><strong>Exit code: </strong><code>{code}</code></p>"
            )?;
        }

        if self.prohibited_mimetype_files.len() > 0 {
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
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
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
        let mut fail_causes: Vec<&str> = self
            .additional_failure_causes
            .iter()
            .map(|s| s.as_str())
            .collect();
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
        if fail_causes.len() > 0 {
            component_spacing(dst, &mut spacing_state)?;
            dst.write_str("**Test failed for the following reasons:**\n")?;
            for s in &fail_causes {
                write!(dst, "\n * {}", s)?;
            }
        }

        if let Some(desc) = &self.description {
            component_spacing(dst, &mut spacing_state)?;
            markdown_write_escaped(dst, desc)?;
        }

        if self.checked_files.len() > 0 {
            component_spacing(dst, &mut spacing_state)?;
            dst.write_str("**The following files were checked for in solution:**\n")?;
            for check_file in &self.checked_files {
                write!(dst, "\n * `{}`", check_file)?;
            }
        }

        if self.mimetype_mismatch_files.len() > 0 {
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
            dst.write_str("### Return Code Mismatch\n\n")?;
            for msg in &mm.msgs {
                write!(dst, "{}\n\n", msg)?;
            }
            write!(dst, "Received return code `{}`. Expected ", mm.received)?;
            if mm.allowed_alternatives.len() == 1 {
                write!(dst, "`{}`.", mm.allowed_alternatives.get(0).unwrap())?;
            } else {
                write!(dst, "one of ")?;
                for (i, a) in mm.allowed_alternatives.iter().enumerate() {
                    if i > 0 {
                        dst.write_str(", ")?;
                    }
                    write!(dst, "`{}`", a)?;
                }
                dst.write_str(".")?;
            }
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
        if fail_causes.len() > 0 {
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

        if self.checked_files.len() > 0 {
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

        if self.mimetype_mismatch_files.len() > 0 {
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
            write!(
                dst,
                "<p><span><strong>Return code:</strong> <code>{code}</code></span></p>"
            )?;
        }

        if let Some(stdin) = &self.stdin_contents {
            dst.write_str("<h6>Standard Input</h6>")?;
            stdin.render_html(settings, dst, escape, header_level + 1)?;
        }

        if let Some(mm) = &self.code_mismatch {
            dst.write_str("<div class=\"mt-3 p-3 border border-2 border-danger rounded\">")?;
            dst.write_str("<h4>Return Code Mismatch</h4>")?;
            for msg in &mm.msgs {
                dst.write_str("<p>")?;
                html_write_str(dst, msg, escape)?;
                dst.write_str("</p>")?;
            }
            write!(
                dst,
                "<span>Received return code <code>{}</code>. Expected ",
                mm.received
            )?;
            match mm.allowed_alternatives.as_slice() {
                &[expected] => {
                    write!(dst, "<code>{expected}</code>.")?;
                }
                many_expected => {
                    dst.write_str("one of ")?;
                    for (i, expected) in many_expected.iter().enumerate() {
                        if i > 0 {
                            dst.write_str(", ")?;
                        }
                        if (i + 1) == many_expected.len() {
                            dst.write_str("or ")?;
                        }
                        write!(dst, "<code>{expected}</code>")?;
                    }
                    dst.write_str(".")?;
                }
            }
            dst.write_str("</span>")?;
            dst.write_str("</div>")?;
        }

        if let Some(mm) = &self.stdout_mismatch {
            mm.render_html(
                settings,
                dst,
                escape,
                header_level + 1,
                "Standard Output Mismatch",
            )?;
        }

        if let Some(mm) = &self.stderr_mismatch {
            mm.render_html(
                settings,
                dst,
                escape,
                header_level + 1,
                "Standard Error Mismatch",
            )?;
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MismatchInfo<A> {
    /// The output that was received from the program
    pub received: A,
    /// The allowed alternatives that this could be set to
    pub allowed_alternatives: Vec<A>,
    /// Optional additional messages
    pub msgs: Vec<String>,
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
        write!(dst, "### {}\n\n", title)?;

        for msg in &self.msgs {
            write!(dst, "{}\n\n", msg)?;
        }
        write!(dst, "**Received {}**:\n\n", output_name)?;

        markdown_write_preformatted_with_truncation(
            dst,
            &self.received,
            Some(settings.markdown.truncate_len),
        )?;

        if self.allowed_alternatives.len() == 1 {
            write!(dst, "\n\n**Expected {}**:\n\n", output_name)?;

            markdown_write_preformatted_with_truncation(
                dst,
                &self.allowed_alternatives.get(0).unwrap(),
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
        dst.write_str("<div class=\"mt-3 p-3 border border-2 border-danger rounded\">")?;
        dst.write_str("<h4>")?;
        html_write_str(dst, title, escape)?;
        dst.write_str("</h4>")?;
        for msg in &self.msgs {
            dst.write_str("<p>")?;
            html_write_str(dst, msg, escape)?;
            dst.write_str("</p>")?;
        }
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
        dst.write_str("</div>")?;

        Ok(())
    }
}

/// Information about a source file to be displayed
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
            dst.write_str(&ex)?;
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
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
        write!(
            dst,
            "`{}` (Identified as MIME-type `{}`",
            self.path, self.mime_identified
        )?;
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
        assert_that!(markdown_write_preformatted(
            &mut dst,
            "int foo() {return 1 < 2;}"
        ))
        .is_ok();
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
        assert_that!(markdown_write_preformatted_with_truncation(
            &mut dst,
            "foo",
            Some(3)
        ))
        .is_ok();
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
        assert_that!(markdown_write_preformatted_with_truncation(
            &mut dst,
            "foo",
            Some(2)
        ))
        .is_ok();
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
            assert_eq!(
                t.known_tag_groups.get("bara-babar"),
                Some(&vec!["babar".to_string()])
            );
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
