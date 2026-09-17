//! One submodule per route of the generated site, each exposing a `body` that
//! returns its content. [`ROUTES`] lists them in sidebar order, and
//! [`render_all`] renders every one into a finished page.

pub mod index;
pub mod rest_api;
pub mod settings;
pub mod test_configuration;

use maud::Markup;

use crate::components::{html_page, Body};
use crate::openapi::Spec;

/// Everything a route body is rendered from.
pub struct RenderOptions<'a> {
    /// The site brand, from the settings file.
    pub name: &'a str,
    /// The OpenAPI spec, as `server emit-openapi` wrote it.
    pub spec: &'a Spec,
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
    pub render_body: fn(&RenderOptions) -> Body,
}

/// Every route of the site, in sidebar order.
pub const ROUTES: &[Route] = &[
    Route {
        file: "index.html",
        nav_label: "Home",
        title: "Documentation",
        description: None,
        render_body: |opts| index::body(opts.name),
    },
    Route {
        file: "settings.html",
        nav_label: "Settings",
        title: "Settings Reference",
        description: Some("the TOML settings file."),
        render_body: |_| settings::body(),
    },
    Route {
        file: "tests.html",
        nav_label: "Test Configuration",
        title: "Test Configuration Reference",
        description: Some("test kinds and defaults."),
        render_body: |_| test_configuration::body(),
    },
    Route {
        file: "api.html",
        nav_label: "REST API",
        title: "REST API Reference",
        description: Some("the HTTP API."),
        render_body: |opts| rest_api::body(opts.spec),
    },
];

/// Renders every route, as `(file name, HTML)`.
pub fn render_all(opts: &RenderOptions) -> Vec<(&'static str, Markup)> {
    let nav: Vec<(&str, &str)> = ROUTES.iter().map(|r| (r.file, r.nav_label)).collect();
    ROUTES
        .iter()
        .enumerate()
        .map(|(i, route)| {
            let body = (route.render_body)(opts);
            (route.file, html_page(opts.name, route.title, &nav, i, body))
        })
        .collect()
}
