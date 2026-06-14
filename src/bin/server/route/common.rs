use sailfish::runtime::Render;

use id2202_autograder::{
    config::Settings,
    reporting::{
        DetailsBuildFailure, DetailsTagGradingGroup, MIMETypeInfo, Report, ReportInvalidTag, ReportMessage, ReportSubmission, ReportTagGrading, ReportWrapper
    },
};

static SAILFISH_HEADER_BAR_ROUTES: [(&str, &str); 2] = [("/", "Home"), ("/job_info", "Job Info")];

pub struct CommonInformation {
    pub header_bar_title: String,
    pub header_bar_routes: [(&'static str, &'static str); 2],
    pub title: String,
    pub current_route: Option<String>,
    pub include_syntax_highlighting: bool,
}

impl CommonInformation {
    pub fn from_title_route(
        settings: &Settings,
        title: &str,
        current_route: &str,
    ) -> CommonInformation {
        CommonInformation {
            header_bar_title: settings.name.clone(),
            header_bar_routes: SAILFISH_HEADER_BAR_ROUTES,
            title: format!("ID2202 | {title}"),
            current_route: Some(current_route.to_string()),
            include_syntax_highlighting: false,
        }
    }

    pub fn from_title(settings: &Settings, title: &str) -> CommonInformation {
        CommonInformation {
            header_bar_title: settings.name.clone(),
            header_bar_routes: SAILFISH_HEADER_BAR_ROUTES,
            title: format!("ID2202 | {title}"),
            current_route: None,
            include_syntax_highlighting: false,
        }
    }
}

// .-------------------------------------------------.
// |  ____                _           _              |
// | |  _ \ ___ _ __   __| | ___ _ __(_)_ __   __ _  |
// | | |_) / _ \ '_ \ / _` |/ _ \ '__| | '_ \ / _` | |
// | |  _ <  __/ | | | (_| |  __/ |  | | | | | (_| | |
// | |_| \_\___|_| |_|\__,_|\___|_|  |_|_| |_|\__, | |
// |                                          |___/  |
// '-------------------------------------------------'

/// A convenient way to only render a string if it has a value.
#[derive(Default)]
pub struct RenderOptionString {
    v: Option<String>,
}

impl Render for RenderOptionString {
    fn render(&self, b: &mut sailfish::runtime::Buffer) -> Result<(), sailfish::RenderError> {
        match &self.v {
            Some(v) => v.render(b),
            None => Ok(()),
        }
    }

    fn render_escaped(
        &self,
        b: &mut sailfish::runtime::Buffer,
    ) -> Result<(), sailfish::RenderError> {
        match &self.v {
            Some(v) => v.render_escaped(b),
            None => Ok(()),
        }
    }
}
impl From<Option<String>> for RenderOptionString {
    fn from(opt: Option<String>) -> Self {
        RenderOptionString { v: opt }
    }
}

/// Wrapper for rendering a report on the submission page. This will format the
/// report assuming that the page is running Bootstrap JS.
pub struct RenderReport {
    pub v: Option<Report>,
    pub options: ReportRenderOptions,
}

pub struct ReportRenderOptions {
    pub symbol_ok: String,
    pub symbol_failed: String,
}

impl Render for RenderReport {
    fn render(&self, b: &mut sailfish::runtime::Buffer) -> Result<(), sailfish::RenderError> {
        match &self.v {
            Some(r) => {
                render_report(r, b, &self.options, false, 3)?;
            }
            None => {
                b.push_str("<p>No report generated.</p>");
            }
        }
        Ok(())
    }

    fn render_escaped(
        &self,
        b: &mut sailfish::runtime::Buffer,
    ) -> Result<(), sailfish::RenderError> {
        match &self.v {
            Some(r) => {
                render_report(r, b, &self.options, true, 3)?;
            }
            None => {
                b.push_str("<p>No report generated.</p>");
            }
        }
        Ok(())
    }
}

/// Push a string to the buffer b on a conditional `escape`.
fn bufpush_str(escape: bool, str: &str, b: &mut sailfish::runtime::Buffer) {
    if escape {
        sailfish::runtime::escape::escape_to_buf(str, b);
    } else {
        b.push_str(str);
    }
}

fn render_report(
    r: &Report,
    b: &mut sailfish::runtime::Buffer,
    options: &ReportRenderOptions,
    escape: bool,
    level: usize,
) -> Result<(), sailfish::RenderError> {
    match r {
        Report::Wrapper(r) => render_report_wrapper(r, b, options, escape, level),
        Report::InvalidTag(r) => render_report_invalid_tag(r, b, options, escape, level),
        Report::Message(r) => render_report_message(r, b, options, escape, level),
        Report::Submission(r) => render_report_submission(r, b, options, escape, level),
        Report::TagGrading(r) => render_report_tag_grading(r, b, options, escape, level),
    }
}

fn render_report_wrapper(
    r: &ReportWrapper,
    b: &mut sailfish::runtime::Buffer,
    options: &ReportRenderOptions,
    escape: bool,
    level: usize,
) -> Result<(), sailfish::RenderError> {
    if let Some(title) = &r.title {
        b.push_str(&format!("<h{level}>"));
        bufpush_str(escape, title, b);
        b.push_str(&format!("</h{level}>"));
    }

    for sub_report in &r.reports {
        b.push_str("<div>");
        render_report(sub_report, b, options, escape, level + 1)?;
        b.push_str("</div>");
    }
    Ok(())
}

fn render_report_invalid_tag(
    r: &ReportInvalidTag,
    b: &mut sailfish::runtime::Buffer,
    _options: &ReportRenderOptions,
    escape: bool,
    level: usize,
) -> Result<(), sailfish::RenderError> {
    b.push_str("<p>Received invalid grading tag: <code>");
    bufpush_str(escape, &r.tag_name, b);
    b.push_str("</code></p>");

    if r.known_grading_tags.len() > 0 {
        b.push_str(&format!("<h{level}>Known Grading Tags</h{level}>"));
        b.push_str("<ul>");
        for t in &r.known_grading_tags {
            b.push_str("<li><code>");
            bufpush_str(escape, &t, b);
            b.push_str("</code></li>");
        }
        b.push_str("</ul>");
    }

    if r.known_tag_groups.len() > 0 {
        b.push_str(&format!("<h{level}>Known Tag Groups</h{level}>"));
        b.push_str("<table class=\"table table-striped table-hover\">");
        b.push_str("<thead><tr>");
        b.push_str("<th scope=\"col\">Group Name</th>");
        b.push_str("<th scope=\"col\">Contained Grading Tags</th>");
        b.push_str("</tr></thead>");
        b.push_str("<tbody>");
        for (groupname, contained_tags) in &r.known_tag_groups {
            b.push_str("<tr>");
            b.push_str("<td><code>");
            bufpush_str(escape, groupname, b);
            b.push_str("</code></td><td>");
            for (i, t) in contained_tags.iter().enumerate() {
                if i > 0 {
                    b.push_str(", ");
                }
                b.push_str("<code>");
                bufpush_str(escape, t, b);
                b.push_str("</code>");
            }
            b.push_str("</td></tr>");
        }
        b.push_str("</tbody></table>");
    }

    Ok(())
}

fn render_report_message(
    r: &ReportMessage,
    b: &mut sailfish::runtime::Buffer,
    _options: &ReportRenderOptions,
    escape: bool,
    _level: usize,
) -> Result<(), sailfish::RenderError> {
    b.push_str("<p>");
    bufpush_str(escape, &r.msg, b);
    b.push_str("</p>");

    Ok(())
}

fn render_report_submission(
    r: &ReportSubmission,
    b: &mut sailfish::runtime::Buffer,
    options: &ReportRenderOptions,
    escape: bool,
    level: usize,
) -> Result<(), sailfish::RenderError> {
    if let Some(reason) = &r.premature_exit_reason {
        b.push_str("<p><em>");
        bufpush_str(escape, reason, b);
        b.push_str("<em></p>");
    }

    for grading_report in &r.tag_reports {
        b.push_str("<div class=\"my-2\">");
        render_report_tag_grading(grading_report, b, options, escape, level)?;
        b.push_str("</div>");
    }

    Ok(())
}

fn render_report_tag_grading(
    r: &ReportTagGrading,
    b: &mut sailfish::runtime::Buffer,
    options: &ReportRenderOptions,
    escape: bool,
    level: usize,
) -> Result<(), sailfish::RenderError> {
    b.push_str(&format!("<h{level}>Results for <code>"));
    bufpush_str(escape, &r.tag_name, b);
    b.push_str(&format!(
        "</code> ({})</h{level}>",
        if r.ok {
            &options.symbol_ok
        } else {
            &options.symbol_failed
        }
    ));

    if r.derived_from.len() > 0 {
        b.push_str("<p><em>(Derived from ");
        for (i, t) in r.derived_from.iter().enumerate() {
            if i > 0 {
                b.push_str(", ");
            }
            b.push_str("<code>");
            bufpush_str(escape, t, b);
            b.push_str("</code>");
        }
        b.push_str(")</em></p>");
    }

    if let Some(bf) = &r.build_failure {
        b.push_str("<div>");
        render_details_build_failure(bf, b, options, escape, level + 1)?;
        b.push_str("</div>");
    }

    if r.groups.len() > 0 {
        b.push_str("<ul>");
        for tgg in &r.groups {
            b.push_str("<li>");
            render_details_tag_grading_group(tgg, b, options, escape, 0)?;
            b.push_str("</li>");
        }
        b.push_str("</ul>");
    }

    // Example of code highlighting with highlightjs
    //b.push_str("<pre><code class=\"language-cpp\">");
    //b.push_str("int main(int argc, char *argv[])\n{\n   return 5;\n}");
    //b.push_str("\n</code></pre>");

    Ok(())
}

fn render_details_build_failure(
    bf: &DetailsBuildFailure,
    b: &mut sailfish::runtime::Buffer,
    options: &ReportRenderOptions,
    escape: bool,
    level: usize,
) -> Result<(), sailfish::RenderError> {
    b.push_str(&format!("<h{level}>Build Failure</h{level}>"));

    b.push_str("<p>");
    bufpush_str(escape, &bf.msg, b);
    b.push_str("</p>");

    if let Some(dir) = &bf.srcdir {
        b.push_str("<p><strong>");
        if bf.missing_source_directory {
            b.push_str("Source directory not found in submission");
        } else {
            b.push_str("Source directory");
        }
        b.push_str(": </strong><code>");
        bufpush_str(escape, dir, b);
        b.push_str("</code></p>");
    }

    if let Some(cmd) = &bf.cmd {
        b.push_str("<p><strong>Build command: </strong><code>");
        bufpush_str(escape, cmd, b);
        b.push_str("</code></p>");
    }

    if let Some(code) = &bf.exit_code {
        b.push_str("<p><strong>Exit code: </strong><code>");
        b.push_str(&code.to_string());
        b.push_str("</code></p>");
    }

    if bf.prohibited_mimetype_files.len() > 0 {
        b.push_str("<p><strong>Prohibited files in your solution:</strong></p>");
        b.push_str("<ul>");
        for mimeinfo in &bf.prohibited_mimetype_files {
            b.push_str("<li>");
            render_mimetype_info(mimeinfo, b, options, escape, level + 1)?;
            b.push_str("</li>");
        }
        b.push_str("</ul>");
    }
    if let Some(sout) = &bf.captured_stdout {
        b.push_str("<p><strong>Captured Standard Output</strong></p>");
        b.push_str("<pre><code>");
        bufpush_str(escape, sout, b);
        b.push_str("</code></pre>");
    }
    if let Some(serr) = &bf.captured_stdout {
        b.push_str("<p><strong>Captured Standard Error</strong></p>");
        b.push_str("<pre><code>");
        bufpush_str(escape, serr, b);
        b.push_str("</code></pre>");
    }

    if let Some(msg) = &bf.suffix_message {
        b.push_str("<p>");
        bufpush_str(escape, msg, b);
        b.push_str("</p>");
    }

    Ok(())
}

fn render_mimetype_info(
    mti: &MIMETypeInfo,
    b: &mut sailfish::runtime::Buffer,
    _options: &ReportRenderOptions,
    escape: bool,
    _level: usize,
) -> Result<(), sailfish::RenderError> {
    b.push_str("<code>");
    bufpush_str(escape, &mti.path, b);
    b.push_str("</code> (Identified as MIME-type <code>");
    bufpush_str(escape, &mti.mime_identified, b);
    if let Some(expected) = &mti.mime_expected {
        b.push_str(", expected <code>");
        bufpush_str(escape, expected, b);
        b.push_str("</code>");
    }
    b.push_str("</code>)");

    Ok(())
}

fn render_details_tag_grading_group(
    tgg: &DetailsTagGradingGroup,
    b: &mut sailfish::runtime::Buffer,
    options: &ReportRenderOptions,
    escape: bool,
    group_level: usize,
) -> Result<(), sailfish::RenderError> {
    b.push_str("<p>");
    if group_level == 0 {
        b.push_str("<strong>");
    }
    bufpush_str(escape, &tgg.group_title, b);
    if group_level == 0 {
        b.push_str("</strong>");
    }
    if tgg.local_tests > 0 {
        if tgg.tests_run < tgg.local_tests {
            b.push_str(&format!(
                " ({}/{} tests run)",
                tgg.tests_run, tgg.local_tests
            ));
        } else {
            b.push_str(&format!(
                " ({}/{} tests passed)",
                tgg.tests_passed, tgg.local_tests
            ));
        }
    }
    b.push_str("</p>");

    if tgg.subgroups.len() > 0 {
        b.push_str("<ul>");
        for sg in &tgg.subgroups {
            b.push_str("<li>");
            render_details_tag_grading_group(sg, b, options, escape, group_level + 1)?;
            b.push_str("</li>");
        }
        b.push_str("</ul>");
    }

    Ok(())
}
