use sailfish::runtime::Render;

use id2202_autograder::{config::Settings, reporting::Report};

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
            title: title.to_string(),
            current_route: Some(current_route.to_string()),
            include_syntax_highlighting: false,
        }
    }

    pub fn from_title(settings: &Settings, title: &str) -> CommonInformation {
        CommonInformation {
            header_bar_title: settings.name.clone(),
            header_bar_routes: SAILFISH_HEADER_BAR_ROUTES,
            title: title.to_string(),
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
pub struct RenderReport<'a> {
    pub v: Option<Report>,
    pub settings: &'a Settings,
}

impl<'a> Render for RenderReport<'a> {
    fn render(&self, b: &mut sailfish::runtime::Buffer) -> Result<(), sailfish::RenderError> {
        match &self.v {
            Some(r) => {
                r.render_html(&self.settings.reporting, b, false, 3)
                    .map_err(|e| {
                        log::error!("could not render report as HTML: {e:?}");
                        sailfish::RenderError::new("error rendering report as HTML")
                    })?;
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
                r.render_html(&self.settings.reporting, b, true, 3)
                    .map_err(|e| {
                        log::error!("could not render report as HTML: {e:?}");
                        sailfish::RenderError::new("error rendering report as HTML")
                    })?;
            }
            None => {
                b.push_str("<p>No report generated.</p>");
            }
        }
        Ok(())
    }
}
