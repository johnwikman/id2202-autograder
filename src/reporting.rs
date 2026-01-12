/// Common reporting interface, used for functions to report in a
/// display-agnostic format, which can then be converted to other formats down
/// the line.
use std::collections::BTreeMap;

use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::{
    config::ReportingMarkdownSettings, error::Error, utils::md_escape,
    utils::md_preformatted_with_truncation,
};

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
    pub fn to_markdown(&self, settings: &ReportingMarkdownSettings) -> String {
        match self {
            Self::Wrapper(r) => r.to_markdown(settings),
            Self::InvalidTag(r) => r.to_markdown(settings),
            Self::Message(r) => r.to_markdown(settings),
            Self::Submission(r) => r.to_markdown(settings),
            Self::TagGrading(r) => r.to_markdown(settings),
        }
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
    pub fn to_markdown(&self, settings: &ReportingMarkdownSettings) -> String {
        let mut parts: Vec<String> = vec![];

        if let Some(t) = &self.title {
            parts.push(t.clone());
        }

        parts.extend(self.reports.iter().map(|r| r.to_markdown(settings)));

        parts.join("\n\n")
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
    /// Format the invalid tag report on a GitHub markdown friendly format
    pub fn to_markdown(&self, _settings: &ReportingMarkdownSettings) -> String {
        let mut md_string: String = format!("Unknown tag: `{}`", self.tag_name);

        if self.known_grading_tags.len() > 0 {
            md_string.push_str("\n\n### Known grading tags\n\n");
            md_string.push_str(
                &self
                    .known_grading_tags
                    .iter()
                    .map(|k| format!("* `{}`", k))
                    .join("\n"),
            );
        }

        if self.known_tag_groups.len() > 0 {
            md_string.push_str("\n\n### Known tag groups\n\n");
            md_string.push_str("| Group Name | Contained Grading Tags |\n");
            md_string.push_str("| ---------- | ---------------------- |\n");
            for (k, tagnames) in &self.known_tag_groups {
                md_string.push_str(&format!(
                    "| `{k}` | {} |\n",
                    tagnames.iter().map(|s| format!("`{s}`")).join(", ")
                ));
            }
            md_string.push_str("\n"); // important with double LF after table
        }

        md_string
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
    pub fn to_markdown(&self, _settings: &ReportingMarkdownSettings) -> String {
        md_escape(&self.msg)
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
    pub fn to_markdown(&self, settings: &ReportingMarkdownSettings) -> String {
        let mut outstr = "# Submission Results".to_string();

        if settings.show_indicator_submission_header {
            outstr.push_str("( ");
            if self.tag_reports.iter().all(|tr| tr.ok) {
                outstr.push_str(&settings.symbol_ok);
            } else {
                outstr.push_str(&settings.symbol_failed);
            }
        }

        if let Some(reason) = &self.premature_exit_reason {
            outstr.push_str("\n\n_(");
            outstr.push_str(reason);
            outstr.push_str(")_");
        }

        // Add the information text
        outstr.push_str("\n\nTests are grouped together into categories.");
        outstr.push_str(" Each category contains a set of test cases that evaluate a specific aspect of your program.");

        outstr.push_str("\n\n * The symbol ");
        outstr.push_str(&settings.symbol_ok);
        outstr.push_str(" indicates that all tests in the category passed.");

        outstr.push_str("\n\n * The symbol ");
        outstr.push_str(&settings.symbol_skipped);
        outstr.push_str(" indicates that not all tests were run in this category.");
        outstr.push_str(" This is usually due to a previous test timeout.");

        outstr.push_str("\n\n * The symbol ");
        outstr.push_str(&settings.symbol_failed);
        outstr.push_str(" indicates that at least one test in the category failed.");

        if let Some(max_details) = self.max_shown_details {
            outstr.push_str("\n\nAdditionally, for the first ");
            if max_details == 1 {
                outstr.push_str("test that fail");
            } else {
                outstr.push_str(&format!("{} tests that fail", max_details));
            }
            outstr
                .push_str(", you will also get more detailed information after the main overview.");
        }

        let mut details = vec![];
        for tr in &self.tag_reports {
            let (s, tr_details) = tr.to_markdown_with_details(settings, details.len());
            outstr.push_str("\n\n");
            outstr.push_str(&s);
            details.extend(tr_details);
        }

        for (i, detail) in details.iter().enumerate() {
            outstr.push_str("\n\n");
            outstr.push_str(&format!("<details id=\"detail-summary-{}\">\n", i + 1));
            outstr.push_str(&format!("<summary>Detail {}</summary>\n\n", i + 1));
            outstr.push_str(&detail.to_markdown(settings));
            outstr.push_str("\n\n</details>");
        }

        outstr
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
    pub fn to_markdown(&self, settings: &ReportingMarkdownSettings) -> String {
        let (mut mdstr, details) = self.to_markdown_with_details(settings, 0);

        for (i, detail) in details.iter().enumerate() {
            mdstr.push_str("\n\n");
            mdstr.push_str(&format!("<details id=\"detail-summary-{}\">\n", i + 1));
            mdstr.push_str(&format!("<summary>Detail {}</summary>\n\n", i + 1));
            mdstr.push_str(&detail.to_markdown(settings));
            mdstr.push_str("\n\n</details>");
        }

        mdstr
    }

    /// Generate markdown with details included separately, to be inserted later.
    fn to_markdown_with_details(
        &self,
        settings: &ReportingMarkdownSettings,
        detail_offset: usize,
    ) -> (String, Vec<DetailsTestFailure>) {
        let mut s = String::new();
        let mut details = vec![];

        s.push_str(&format!("## Results for tag `{}`", &self.tag_name));
        if settings.show_indicator_tag_header {
            if self.ok {
                s.push_str(&format!(" ({})", settings.symbol_ok));
            } else {
                s.push_str(&format!(" ({})", settings.symbol_failed));
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
            s.push_str(&format!(
                "\n\n_(Derived from {})_",
                derivs.iter().map(|s| format!("`{}`", s)).join(", ")
            ));
        }

        s.push_str("\n\n");

        if let Some(bs) = &self.build_failure {
            s.push_str(&bs.to_markdown(settings));
        } else {
            let mut tg_str = String::new();
            let mut all_run = true;
            let mut all_ok = true;
            for tg in &self.groups {
                let (g_all_run, g_all_ok, g_str, g_details) =
                    tg.to_markdown_with_details(settings, detail_offset + details.len(), 0);
                all_run &= g_all_run;
                all_ok &= g_all_ok;
                tg_str.push_str(&g_str);
                details.extend(g_details);
            }
            if !all_run {
                s.push_str("Grading process was interrupted.");
            } else if !all_ok {
                s.push_str("Some test cases failed.");
            } else {
                s.push_str(&format!(
                    "All test cases passed for this tag! {}",
                    settings.symbol_tagsuccess
                ));
            }
            if details.len() > 0 {
                s.push_str(" See details below.");
            }

            s.push_str("\n\n");
            s.push_str(&tg_str);
        }

        (s, details)
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

impl DetailsTagGradingGroup {
    /// Generates the test results within a grading tag.
    ///
    /// Note: the generated string will always terminate with a newline.
    fn to_markdown_with_details(
        &self,
        settings: &ReportingMarkdownSettings,
        detail_offset: usize,
        indent: usize,
    ) -> (bool, bool, String, Vec<DetailsTestFailure>) {
        let mut outstr = String::new();
        outstr.extend(std::iter::repeat(' ').take(indent));
        outstr.push_str(" * ");

        // Use number of tests that have passed as an indicator
        let mut all_run = self.local_tests == self.tests_run;
        let mut all_ok = self.local_tests == self.tests_passed;
        let mut followstr = String::new();
        let mut details = self.test_details.clone();
        for sg in &self.subgroups {
            let (sg_all_run, sg_all_ok, sg_str, sg_details) =
                sg.to_markdown_with_details(settings, detail_offset + details.len(), indent + 2);
            all_run &= sg_all_run;
            all_ok &= sg_all_ok;
            followstr.extend(sg_str.chars());
            details.extend(sg_details.into_iter());
        }

        if !all_run {
            outstr.push_str(&settings.symbol_skipped);
        } else if !all_ok {
            outstr.push_str(&settings.symbol_failed)
        } else {
            outstr.push_str(&settings.symbol_ok);
        }
        outstr.push(' ');
        // Bold face title if we are on top-level
        if indent == 0 {
            outstr.push_str("**");
        }
        outstr.push_str(&self.group_title);
        if indent == 0 {
            outstr.push_str("**");
        }
        if self.local_tests > 0 {
            if self.tests_run < self.local_tests {
                outstr.push_str(&format!(
                    " ({}/{} tests run)",
                    self.tests_run, self.local_tests
                ));
            } else {
                outstr.push_str(&format!(
                    " ({}/{} tests passed)",
                    self.tests_passed, self.local_tests
                ));
            }
        }
        if self.test_details.len() > 0 {
            outstr.push_str(&format!(
                "\n{}   [{}]",
                " ".repeat(indent),
                (0..self.test_details.len())
                    .map(|i| format!("<a href=\"#detail-summary-{}\">Detail {}</a>", i + 1, i + 1))
                    .join(", ")
            ));
        }
        outstr.push('\n');
        outstr.push_str(&followstr);

        (all_run, all_ok, outstr, details)
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
    pub fn to_markdown(&self, settings: &ReportingMarkdownSettings) -> String {
        let mut outstr: String = format!("{} {}", settings.symbol_build, self.msg);

        if let Some(dir) = &self.srcdir {
            outstr.push_str("\n\n");
            outstr.push_str(&format!("**Source directory:** `{}`", dir));
        }
        if let Some(cmd) = &self.cmd {
            outstr.push_str("\n\n");
            outstr.push_str(&format!("**Build command:** `{}`", cmd));
        }
        if let Some(code) = &self.exit_code {
            outstr.push_str("\n\n");
            outstr.push_str(&format!("**Exit code:** `{}`", code));
        }
        if self.missing_source_directory {
            outstr.push_str("\n\n**The expected source directory is missing in your submission.**");
        }
        if self.prohibited_mimetype_files.len() > 0 {
            outstr.push_str("\n\n**Prohibited files in your solution:**\n");
            for mimeinfo in &self.prohibited_mimetype_files {
                outstr.push_str(&format!("\n * {}", mimeinfo.to_markdown(settings)));
            }
        }
        if let Some(sout) = &self.captured_stdout {
            outstr.push_str("\n\n### Captured Standard Output\n\n");
            outstr.push_str(&md_preformatted_with_truncation(
                sout,
                Some(settings.truncate_len),
            ));
        }
        if let Some(serr) = &self.captured_stdout {
            outstr.push_str("\n\n### Captured Standard Error\n\n");
            outstr.push_str(&md_preformatted_with_truncation(
                serr,
                Some(settings.truncate_len),
            ));
        }

        if let Some(msg) = &self.suffix_message {
            outstr.push_str(msg);
        }

        outstr
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
    fn to_markdown(&self, settings: &ReportingMarkdownSettings) -> String {
        let mut parts: Vec<String> = vec![];

        // Summarize failure causes
        let mut fail_causes = self.additional_failure_causes.clone();
        if self.code_mismatch.is_some() {
            fail_causes.push("Return code mismatch.".to_string());
        }
        if self.stdout_mismatch.is_some() {
            fail_causes.push("Standard output mismatch.".to_string());
        }
        if self.stderr_mismatch.is_some() {
            fail_causes.push("Standard error mismatch.".to_string());
        }
        if fail_causes.len() > 0 {
            let mut failstr = "**Test failed for the following reasons:**\n".to_string();
            for s in &fail_causes {
                failstr.push_str("\n * ");
                failstr.push_str(s);
            }
            parts.push(failstr);
        }

        if let Some(desc) = &self.description {
            parts.push(desc.clone());
        }

        if self.checked_files.len() > 0 {
            let mut s = "**The following files were checked for in solution:**\n".to_string();
            for check_file in &self.checked_files {
                s.push_str(&format!("\n * `{}`", check_file));
            }
            parts.push(s);
        }

        if self.mimetype_mismatch_files.len() > 0 {
            let mut s = "**MIME-type mismatches in your solution:**\n".to_string();
            for mimeinfo in &self.mimetype_mismatch_files {
                s.push_str(&format!("\n * {}", mimeinfo.to_markdown(settings)));
            }
            parts.push(s);
        }

        if let Some(cmd) = &self.command {
            parts.push(format!("**Command:** `{}`", cmd));
        }

        if let Some(code) = &self.code_captured {
            parts.push(format!("**Return code:** `{}`", code));
        }

        if let Some(stdin) = &self.stdin_contents {
            let mut s = "### Standard Input\n\n".to_string();
            s.push_str(&stdin.to_markdown(settings));
            parts.push(s);
        }

        for (i, infile) in self.input_file_contents.iter().enumerate() {
            let mut s = "### Input File".to_string();
            if self.input_file_contents.len() > 1 {
                s.push_str(&format!(" {}", i + 1));
            }
            s.push_str("\n\n");
            s.push_str(&infile.to_markdown(settings));
            parts.push(s);
        }

        if let Some(mm) = &self.code_mismatch {
            let mut s = "### Return Code Mismatch\n\n".to_string();
            for msg in &mm.msgs {
                s.push_str(msg);
                s.push_str("\n\n");
            }
            s.push_str(&format!(
                "Received return code `{}`. Expected ",
                mm.received
            ));
            if mm.allowed_alternatives.len() == 1 {
                s.push_str(&format!("`{}`.", mm.allowed_alternatives.get(0).unwrap()));
            } else {
                s.push_str(&format!(
                    "one of {}.",
                    mm.allowed_alternatives
                        .iter()
                        .map(|a| format!("`{}`", a))
                        .join(", ")
                ));
            }
            parts.push(s);
        }

        if let Some(mm) = &self.stdout_mismatch {
            parts.push(mm.to_markdown(settings, "Standard Output Mismatch", "stdout"));
        }
        if let Some(mm) = &self.stderr_mismatch {
            parts.push(mm.to_markdown(settings, "Standard Error Mismatch", "stderr"));
        }

        for (title, file_info) in &self.additional_files {
            parts.push(format!(
                "### {}\n\n{}",
                title,
                file_info.to_markdown(settings)
            ));
        }

        if let Some(cap) = &self.stdout_captured {
            let mut s = "### Captured Standard Output\n\n".to_string();
            s.push_str(&md_preformatted_with_truncation(
                cap,
                Some(settings.truncate_len),
            ));
            parts.push(s);
        }

        if let Some(cap) = &self.stderr_captured {
            let mut s = "### Captured Standard Error\n\n".to_string();
            s.push_str(&md_preformatted_with_truncation(
                cap,
                Some(settings.truncate_len),
            ));
            parts.push(s);
        }

        parts.join("\n\n")
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
    fn to_markdown(
        &self,
        settings: &ReportingMarkdownSettings,
        title: &str,
        output_name: &str,
    ) -> String {
        let mut outstr = format!("### {}\n\n", title);

        for msg in &self.msgs {
            outstr.push_str(msg);
            outstr.push_str("\n\n");
        }
        outstr.push_str(&format!("**Received {}**:\n\n", output_name));

        outstr.push_str(&md_preformatted_with_truncation(
            &self.received,
            Some(settings.truncate_len),
        ));

        if self.allowed_alternatives.len() == 1 {
            outstr.push_str(&format!("**Expected {}**:\n\n", output_name));
            outstr.push_str(&md_preformatted_with_truncation(
                &self.allowed_alternatives.get(0).unwrap(),
                Some(settings.truncate_len),
            ));
        } else {
            outstr.push_str("**Expected one of**:\n\n");
            outstr.push_str(
                &self
                    .allowed_alternatives
                    .iter()
                    .map(|s| md_preformatted_with_truncation(s, Some(settings.truncate_len)))
                    .join("\n\n**or**\n\n"),
            );
        }

        outstr
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
    fn to_markdown(&self, _settings: &ReportingMarkdownSettings) -> String {
        let mut outstr = "```".to_string();
        if let Some(ex) = &self.extension {
            outstr.push_str(&ex);
        }
        outstr.push_str("\n");
        outstr.push_str(&self.content);
        outstr.push_str("\n```");
        outstr
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
    fn to_markdown(&self, _settings: &ReportingMarkdownSettings) -> String {
        let mut s = format!(
            "`{}` (Identified as MIME-type `{}`",
            self.path, self.mime_identified
        );
        if let Some(expected) = &self.mime_expected {
            s.push_str(", `");
            s.push_str(expected);
            s.push_str("`");
        }
        s.push(')');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asserting::prelude::*;

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
