//! The site landing page.

use crate::html::{html_page, Body};
use crate::markdown::escape;

pub fn render(name: &str) -> String {
    let body = Body::new(&format!(
        "<h1>{} Documentation</h1>\n\
        <p>Reference documentation for operating the autograder, generated \
        directly from the source so it stays in sync with the code. Use the \
        navigation on the left, or the links below:</p>\n\
        <ul>\n\
        <li><a href=\"settings.html\">Settings Reference</a> — the TOML settings file.</li>\n\
        <li><a href=\"tests.html\">Test Configuration Reference</a> — test kinds and defaults.</li>\n\
        <li><a href=\"api.html\">REST API Reference</a> — the HTTP API.</li>\n\
        </ul>\n",
        escape(name)
    ));
    html_page(name, "Documentation", "index.html", body)
}
