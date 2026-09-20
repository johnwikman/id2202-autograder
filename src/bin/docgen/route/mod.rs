//! One submodule per route of the generated site, each exposing a `body` that
//! returns its content. [`ROUTES`] lists them in sidebar order, and
//! [`render_all`] renders every one into a finished page.

pub mod index;
pub mod rest_api;
pub mod settings;
pub mod test_configuration;
pub mod autograder_verifier_tools;

use maud::Markup;

use crate::components::{html_page, Body, NavItem};
use crate::griffe::Dump;
use crate::openapi::Spec;

/// Everything a route body is rendered from.
pub struct RenderOptions<'a> {
    /// The site brand, from the settings file.
    pub name: &'a str,
    /// The OpenAPI spec, as `server emit-openapi` wrote it.
    pub spec: &'a Spec,
    /// The verifier tools package, as `griffe dump` wrote it.
    pub dump: &'a Dump,
}

/// One route of the generated site: everything written about it outside its own
/// body, and the renderer for that body.
pub struct Route {
    /// File name it is written to, and linked to by.
    pub file: &'static str,
    /// Sidebar label.
    pub nav_label: &'static str,
    /// `<title>`, and the link text on the landing page.
    pub title: &'static str,
    /// The line shown after the link on the landing page. A route with `None`
    /// is not listed there at all.
    pub description: Option<&'static str>,
    /// The collapsible sidebar heading this route is listed under, or `None`
    /// to list it at the top level.
    pub group: Option<&'static str>,
    pub render_body: fn(&RenderOptions) -> Body,
}

/// Every route of the site, in sidebar order. A grouped route is listed after
/// the ungrouped ones whatever its place here.
pub const ROUTES: &[Route] = &[
    Route {
        file: "index.html",
        nav_label: "Home",
        title: "Documentation",
        description: None,
        group: None,
        render_body: |opts| index::body(opts.name),
    },
    Route {
        file: "settings.html",
        nav_label: "Settings",
        title: "Settings Reference",
        description: Some("the TOML settings file, general settings for the autograder."),
        group: None,
        render_body: |_| settings::body(),
    },
    Route {
        file: "tests.html",
        nav_label: "Test Configuration",
        title: "Test Configuration Reference",
        description: Some("test kinds and defaults."),
        group: None,
        render_body: |_| test_configuration::body(),
    },
    Route {
        file: "api.html",
        nav_label: "REST API",
        title: "REST API Reference",
        description: Some("the HTTP API."),
        group: None,
        render_body: |opts| rest_api::body(opts.spec),
    },
    Route {
        file: "autograder-verifier-tools.html",
        nav_label: "Autograder Verifier Tools",
        title: "Autograder Verifier Tools Reference",
        description: Some("the optional Python helper library for run_verifier verifiers."),
        group: Some("Misc"),
        render_body: |opts| autograder_verifier_tools::body(opts.dump),
    },
];

/// Renders every route, as `(file name, HTML)`.
pub fn render_all(opts: &RenderOptions) -> Vec<(&'static str, Markup)> {
    let nav: Vec<NavItem> = ROUTES
        .iter()
        .map(|r| NavItem { file: r.file, label: r.nav_label, group: r.group })
        .collect();
    ROUTES
        .iter()
        .map(|route| {
            let body = (route.render_body)(opts);
            (route.file, html_page(opts.name, route.title, &nav, route.file, body))
        })
        .collect()
}
