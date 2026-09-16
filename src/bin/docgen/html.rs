//! The shared HTML shell and the small Bootstrap widgets the pages are built
//! from.
//!
//! Every generated page is wrapped in one common shell — a left navigation bar
//! (with the site brand at the top) listing all pages — so the whole set reads
//! as a single site. Styling uses the same Bootstrap version as the autograder's
//! web UI (see `web/templates`); the page-specific rules live in `docs.css`.

use crate::highlight;
use crate::markdown::{self, escape};

/// All documentation pages, as `(href, label)`, in navigation order. Adding a
/// page here makes it appear in the sidebar of every generated page.
pub const NAV: &[(&str, &str)] = &[
    ("index.html", "Home"),
    ("settings.html", "Settings"),
    ("tests.html", "Test Configuration"),
    ("api.html", "REST API"),
];

/// The relative directory (under the output dir) that the cached CDN assets are
/// written to, and referenced from in the generated pages.
pub const VENDOR_DIR: &str = "vendor";

/// Page-specific styling, inlined into every page's `<style>` block.
const DOC_CSS: &str = include_str!("docs.css");

/// The colour-mode toggler the web UI uses (see `web/templates/header.stpl`),
/// inlined into `<head>` so the stored theme is applied before the first paint.
/// It reads `<button data-bs-theme-value>` and the `#bd-theme` dropdown below.
const COLOR_SCHEME_JS: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/web/static/script/color-scheme.js"));

/// The icons the theme dropdown is built from, as an inline sprite (the web UI
/// inlines the same four symbols). Referenced by `<use href="#...">`.
const ICONS: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" class="d-none">
  <symbol id="circle-half" viewBox="0 0 16 16"><path d="M8 15A7 7 0 1 0 8 1v14zm0 1A8 8 0 1 1 8 0a8 8 0 0 1 0 16z"></path></symbol>
  <symbol id="sun-fill" viewBox="0 0 16 16"><path d="M8 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM8 0a.5.5 0 0 1 .5.5v2a.5.5 0 0 1-1 0v-2A.5.5 0 0 1 8 0zm0 13a.5.5 0 0 1 .5.5v2a.5.5 0 0 1-1 0v-2A.5.5 0 0 1 8 13zm8-5a.5.5 0 0 1-.5.5h-2a.5.5 0 0 1 0-1h2a.5.5 0 0 1 .5.5zM3 8a.5.5 0 0 1-.5.5h-2a.5.5 0 0 1 0-1h2A.5.5 0 0 1 3 8zm10.657-5.657a.5.5 0 0 1 0 .707l-1.414 1.415a.5.5 0 1 1-.707-.708l1.414-1.414a.5.5 0 0 1 .707 0zm-9.193 9.193a.5.5 0 0 1 0 .707L3.05 13.657a.5.5 0 0 1-.707-.707l1.414-1.414a.5.5 0 0 1 .707 0zm9.193 2.121a.5.5 0 0 1-.707 0l-1.414-1.414a.5.5 0 0 1 .707-.707l1.414 1.414a.5.5 0 0 1 0 .707zM4.464 4.465a.5.5 0 0 1-.707 0L2.343 3.05a.5.5 0 1 1 .707-.707l1.414 1.414a.5.5 0 0 1 0 .708z"></path></symbol>
  <symbol id="moon-stars-fill" viewBox="0 0 16 16"><path d="M6 .278a.768.768 0 0 1 .08.858 7.208 7.208 0 0 0-.878 3.46c0 4.021 3.278 7.277 7.318 7.277.527 0 1.04-.055 1.533-.16a.787.787 0 0 1 .81.316.733.733 0 0 1-.031.893A8.349 8.349 0 0 1 8.344 16C3.734 16 0 12.286 0 7.71 0 4.266 2.114 1.312 5.124.06A.752.752 0 0 1 6 .278z"></path><path d="M10.794 3.148a.217.217 0 0 1 .412 0l.387 1.162c.173.518.579.924 1.097 1.097l1.162.387a.217.217 0 0 1 0 .412l-1.162.387a1.734 1.734 0 0 0-1.097 1.097l-.387 1.162a.217.217 0 0 1-.412 0l-.387-1.162A1.734 1.734 0 0 0 9.31 6.593l-1.162-.387a.217.217 0 0 1 0-.412l1.162-.387a1.734 1.734 0 0 0 1.097-1.097l.387-1.162zM13.863.099a.145.145 0 0 1 .274 0l.258.774c.115.346.386.617.732.732l.774.258a.145.145 0 0 1 0 .274l-.774.258a1.156 1.156 0 0 0-.732.732l-.258.774a.145.145 0 0 1-.274 0l-.258-.774a1.156 1.156 0 0 0-.732-.732l-.774-.258a.145.145 0 0 1 0-.274l.774-.258c.346-.115.617-.386.732-.732L13.863.1z"></path></symbol>
  <symbol id="check2" viewBox="0 0 16 16"><path d="M13.854 3.646a.5.5 0 0 1 0 .708l-7 7a.5.5 0 0 1-.708 0l-3.5-3.5a.5.5 0 1 1 .708-.708L6.5 10.293l6.646-6.647a.5.5 0 0 1 .708 0z"></path></symbol>
  <!-- (https://icons.getbootstrap.com/icons/exclamation-triangle-fill/) -->
  <symbol id="exclamation-triangle-fill" viewBox="0 0 16 16"><path d="M8.982 1.566a1.13 1.13 0 0 0-1.96 0L.165 13.233c-.457.778.091 1.767.98 1.767h13.713c.889 0 1.438-.99.98-1.767zM8 5c.535 0 .954.462.9.995l-.35 3.507a.552.552 0 0 1-1.1 0L7.1 5.995A.905.905 0 0 1 8 5m.002 6a1 1 0 1 1 0 2 1 1 0 0 1 0-2"></path></symbol>
  <!-- (https://icons.getbootstrap.com/icons/info-circle-fill/) -->
  <symbol id="info-circle-fill" viewBox="0 0 16 16"><path d="M8 16A8 8 0 1 0 8 0a8 8 0 0 0 0 16m.93-9.412-1 4.705c-.07.34.029.533.304.533.194 0 .487-.07.686-.246l-.088.416c-.287.346-.92.598-1.465.598-.703 0-1.002-.422-.808-1.319l.738-3.468c.064-.293.006-.399-.287-.47l-.451-.081.082-.381 2.29-.287zM8 5.5a1 1 0 1 1 0-2 1 1 0 0 1 0 2"></path></symbol>
  <symbol id="lock-fill" viewBox="0 0 16 16"><path d="M8 1a2 2 0 0 1 2 2v4H6V3a2 2 0 0 1 2-2m3 6V3a3 3 0 0 0-6 0v4a2 2 0 0 0-2 2v5a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2"></path></symbol>
</svg>
"##;

/// Marks an authenticated endpoint or a security scheme. Uses the sprite above
/// rather than 🔒, which renders as a blank box wherever no emoji font is
/// installed.
pub const LOCK_ICON: &str =
    r##"<svg class="bi doc-icon-inline" aria-hidden="true"><use href="#lock-fill"></use></svg>"##;

/// Opens the accordion item a `#fragment` points at, so a submenu link to an
/// endpoint reveals it instead of landing on a collapsed row. Covers a click
/// (which changes the fragment) and a page opened on one. A page with no
/// accordion has nothing to match and does nothing.
const REVEAL_JS: &str = r##"(() => {
  "use strict";
  const reveal = () => {
    const target = location.hash ? document.getElementById(location.hash.slice(1)) : null;
    const panel = target && target.querySelector(".accordion-collapse");
    if (panel && !panel.classList.contains("show")) {
      bootstrap.Collapse.getOrCreateInstance(panel).show();
    }
  };
  window.addEventListener("hashchange", reveal);
  reveal();
})();
"##;

/// The Light/Dark/Auto dropdown, driven by [`COLOR_SCHEME_JS`]. Sits at the
/// bottom of the sidebar and opens upwards, so the menu stays on screen.
const THEME_PICKER: &str = r##"<div class="dropup doc-theme mt-auto pt-2 border-top">
      <button id="bd-theme" class="doc-theme-btn btn btn-link nav-link dropdown-toggle d-flex w-100 align-items-center justify-content-center" type="button" data-bs-toggle="dropdown" data-bs-display="static" aria-expanded="false" aria-label="Toggle theme (auto)">
        <svg class="bi theme-icon-active" aria-hidden="true"><use href="#circle-half"></use></svg>
        <span class="ms-2" id="bd-theme-text">Toggle theme</span>
      </button>
      <ul class="doc-theme-menu dropdown-menu" aria-labelledby="bd-theme-text">
        <li><button type="button" class="dropdown-item d-flex align-items-center" data-bs-theme-value="light" aria-pressed="false">
          <svg class="bi me-2 opacity-50" aria-hidden="true"><use href="#sun-fill"></use></svg>Light
          <svg class="bi ms-auto d-none" aria-hidden="true"><use href="#check2"></use></svg></button></li>
        <li><button type="button" class="dropdown-item d-flex align-items-center" data-bs-theme-value="dark" aria-pressed="false">
          <svg class="bi me-2 opacity-50" aria-hidden="true"><use href="#moon-stars-fill"></use></svg>Dark
          <svg class="bi ms-auto d-none" aria-hidden="true"><use href="#check2"></use></svg></button></li>
        <li><button type="button" class="dropdown-item d-flex align-items-center active" data-bs-theme-value="auto" aria-pressed="true">
          <svg class="bi me-2 opacity-50" aria-hidden="true"><use href="#circle-half"></use></svg>Auto
          <svg class="bi ms-auto d-none" aria-hidden="true"><use href="#check2"></use></svg></button></li>
      </ul>
    </div>
"##;

/// A bordered box whose heading is a filled badge sitting on the top-left
/// border, so the heading takes only the width it needs and reads as part of
/// the frame. `title` is inserted verbatim, so callers escape their own text.
pub fn notched_box(title: &str, inner_html: &str) -> String {
    format!(
        "<div class=\"border rounded-3 p-3 pt-4 mt-3 mb-3 position-relative bg-secondary-subtle\">\
         <span class=\"position-absolute top-0 start-0 translate-middle-y ms-3 badge text-bg-secondary\">\
         {title}</span>\n{inner_html}</div>\n"
    )
}

/// A collapsed disclosure block, framed as a card whose summary is a full-width
/// header bar (see `.doc-details` in `docs.css`), so it reads as something to
/// click rather than as a line of text. `summary` is the (escaped) clickable
/// label; `inner_html` is inserted verbatim and hidden until the block is
/// opened.
pub fn details(summary: &str, inner_html: &str) -> String {
    format!(
        "<details class=\"doc-details mb-3\">\
         <summary class=\"doc-summary\">{}</summary>\n{inner_html}</details>\n",
        escape(summary)
    )
}

pub fn code_block(code: &str, lang: &str) -> String {
    format!(
        "<pre class=\"doc-code-block\"><code>{}</code></pre>\n",
        highlight::highlight(code, lang)
    )
}

/// Builds a Bootstrap-styled HTML table. `headers` are plain text (escaped
/// here); each cell in `rows` is expected to already be valid HTML. Every
/// column but the last is marked `doc-cell-narrow`: they hold short labels,
/// and the last one holds the prose the table is read for.
pub fn html_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let narrow = |i: usize, len: usize| match i + 1 < len {
        true => " class=\"doc-cell-narrow\"",
        false => "",
    };
    let mut out =
        String::from("<div class=\"table-responsive\">\n<table class=\"table doc-table\">\n<thead><tr>");
    for (i, h) in headers.iter().enumerate() {
        out.push_str(&format!("<th{}>{}</th>", narrow(i, headers.len()), escape(h)));
    }
    out.push_str("</tr></thead>\n<tbody>\n");
    for row in rows {
        out.push_str("<tr>");
        for (i, cell) in row.iter().enumerate() {
            out.push_str(&format!("<td{}>{cell}</td>", narrow(i, row.len())));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</tbody>\n</table>\n</div>\n");
    out
}

/// An entry in the sidebar submenu, recorded as the body is written.
struct Heading {
    /// 2..=5; each level past the first is indented one step in the submenu.
    level: usize,
    id: String,
    /// Inner HTML, reused verbatim as the submenu label.
    label: String,
}

/// A page body under construction. Headings go through [`Body::heading`] (or
/// come out of [`Body::markdown`], bar the section headings it sets as callouts),
/// which gives each one an id, a link to itself, and an entry in the sidebar
/// submenu — so the submenu is built from what the page actually wrote rather
/// than by reading the finished HTML back.
#[derive(Default)]
pub struct Body {
    html: String,
    toc: Vec<Heading>,
}

impl Body {
    pub fn new(html: &str) -> Self {
        Body { html: html.to_string(), toc: Vec::new() }
    }

    /// HTML inserted verbatim. Any heading in it stays out of the submenu.
    pub fn raw(&mut self, html: &str) {
        self.html.push_str(html);
    }

    /// A heading at `level`, whose inner HTML is inserted verbatim.
    pub fn heading(&mut self, level: usize, inner_html: &str) {
        let html = self.heading_html(level, inner_html, "");
        self.html.push_str(&html);
    }

    /// A heading at `level` that is nothing but the name of a thing — a TOML
    /// table, a test kind — which `docs.css` sets apart from a heading that
    /// only mentions one.
    pub fn name_heading(&mut self, level: usize, name: &str) {
        let inner = format!("<code class=\"doc-name\">{}</code>", escape(name));
        let html = self.heading_html(level, &inner, " doc-heading-name");
        self.html.push_str(&html);
    }

    /// Markdown, whose own headings are treated as [`Body::heading`] does,
    /// bar the rustdoc sections [`doc_markdown`] boxes.
    pub fn markdown(&mut self, src: &str) {
        let html = doc_markdown(src, "", &mut |level, inner| self.heading_html(level, inner, ""));
        self.html.push_str(&html);
    }

    /// A submenu entry for something that is not a heading — the REST API
    /// endpoints, which are accordion headers. `id` has to exist in the body.
    pub fn entry(&mut self, level: usize, id: &str, label: &str) {
        self.toc.push(Heading { level, id: id.to_string(), label: label.to_string() });
    }

    /// The heading markup, with a unique id, the classes `docs.css` styles a
    /// heading by (`extra_class` on top of them), and the anchor link revealed
    /// on hover, recording the submenu entry on the way.
    fn heading_html(&mut self, level: usize, inner_html: &str, extra_class: &str) -> String {
        let mut id = slug(inner_html);
        if id.is_empty() {
            id = format!("section-{}", self.toc.len() + 1);
        }
        while self.toc.iter().any(|h| h.id == id) {
            id.push('-');
        }
        let out = format!(
            "<h{level} id=\"{id}\" class=\"doc-heading doc-h{level}{extra_class}\">{inner_html}\
             <a class=\"doc-anchor\" href=\"#{id}\" aria-label=\"Link to this section\">#</a>\
             </h{level}>\n"
        );
        self.toc.push(Heading { level, id, label: inner_html.to_string() });
        out
    }
}

/// Markdown from a doc comment: a heading naming a rustdoc section (e.g.
/// `# Warning` or `# Note`) is set, together with everything up to the next
/// heading, as a callout box; every other heading is handed to `heading` as
/// [`markdown::markdown`] does. `box_class` is put on every callout box, for a
/// caller that needs them spaced differently from a page's own.
pub fn doc_markdown(
    src: &str,
    box_class: &str,
    heading: &mut dyn FnMut(usize, &str) -> String,
) -> String {
    let mut out = String::new();
    let mut prose = String::new();
    let mut lines = src.lines().peekable();
    while let Some(line) = lines.next() {
        let kind = markdown::heading_text(line).and_then(|t| Some((t, callout_kind(t)?)));
        let Some((label, (class, icon))) = kind else {
            prose.push_str(line);
            prose.push('\n');
            continue;
        };
        out.push_str(&markdown::markdown(&prose, &mut *heading));
        prose.clear();
        let mut section = String::new();
        while let Some(next) = lines.next_if(|l| markdown::heading_text(l).is_none()) {
            section.push_str(next);
            section.push('\n');
        }
        let inner = markdown::markdown(&section, &mut *heading);
        let classes = format!("{class} {box_class}");
        out.push_str(&callout(label, classes.trim_end(), icon, &inner));
    }
    out.push_str(&markdown::markdown(&prose, heading));
    out
}

/// The colour class and icon a rustdoc section heading is drawn with, or `None`
/// for a heading that opens a section of the page and is left as one.
fn callout_kind(label: &str) -> Option<(&'static str, &'static str)> {
    match label.trim().to_lowercase().as_str() {
        "panics" => Some(("cal-danger", "exclamation-triangle-fill")),
        "warning" | "safety" => Some(("cal-warning", "exclamation-triangle-fill")),
        "note" | "info" => Some(("cal-info", "info-circle-fill")),
        "default" | "important" => Some(("cal-note", "info-circle-fill")),
        _ => None,
    }
}

/// A callout box (see `.doc-callout` in `docs.css`): a tinted bar carrying
/// `icon` and `label`, with `inner_html` inserted verbatim below it.
fn callout(label: &str, class: &str, icon: &str, inner_html: &str) -> String {
    format!(
        "<div class=\"doc-callout {class}\">\
         <div class=\"doc-callout-label\">\
         <svg class=\"bi\" aria-hidden=\"true\"><use href=\"#{icon}\"></use></svg>{}</div>\n\
         <div class=\"doc-callout-body\">{inner_html}</div></div>\n",
        escape(label)
    )
}

/// Turns a heading's inner HTML into an anchor id: tags dropped, lowercased,
/// and every run of non-alphanumeric characters collapsed to a single dash.
pub fn slug(label: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in label.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if in_tag => {}
            _ if c.is_ascii_alphanumeric() => out.push(c.to_ascii_lowercase()),
            _ if !out.ends_with('-') => out.push('-'),
            _ => {}
        }
    }
    out.trim_matches('-').to_string()
}

/// Wraps a page body in the shared site shell: the Bootstrap left sidebar (with
/// `name` as the brand at the top and the page matching `active_href`
/// highlighted) and the content area. The entries the body recorded are listed
/// as links under its sidebar entry; a body that recorded none gets no submenu.
pub fn html_page(name: &str, title: &str, active_href: &str, body: Body) -> String {
    let Body { html: body, toc } = body;

    let mut nav = String::new();
    for (href, label) in NAV {
        let active = *href == active_href;
        let class = if active { "nav-link active" } else { "nav-link" };
        nav.push_str(&format!(
            "<li class=\"nav-item\"><a class=\"{class}\" href=\"{}\">{}</a>",
            escape(href),
            escape(label)
        ));
        if active && !toc.is_empty() {
            nav.push_str("\n<ul class=\"nav flex-column doc-toc ms-3\">\n");
            for h in &toc {
                let indent = if h.level > 2 { format!(" lvl-{}", h.level) } else { String::new() };
                nav.push_str(&format!(
                    "<li class=\"nav-item\"><a class=\"nav-link py-1{indent}\" href=\"#{}\">{}</a></li>\n",
                    h.id, h.label
                ));
            }
            nav.push_str("</ul>\n");
        }
        nav.push_str("</li>\n");
    }

    format!(
        r##"<!doctype html>
<html lang="en" data-bs-theme="light">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — {brand} Documentation</title>
<link href="{VENDOR_DIR}/npm/bootstrap@5.3.3/dist/css/bootstrap.min.css" rel="stylesheet">
<script>{COLOR_SCHEME_JS}</script>
<style>
{DOC_CSS}
{highlight_css}
</style>
</head>
<body>
{ICONS}
<nav class="navbar bg-body-tertiary border-bottom sticky-top d-lg-none">
  <div class="container-fluid">
    <button class="navbar-toggler" type="button" data-bs-toggle="offcanvas" data-bs-target="#doc-nav" aria-controls="doc-nav" aria-label="Toggle navigation">
      <span class="navbar-toggler-icon"></span>
    </button>
    <a class="navbar-brand fw-bold ms-3 me-auto" href="index.html">{brand}</a>
  </div>
</nav>
<div class="d-flex">
  <nav class="doc-sidebar offcanvas-lg offcanvas-start bg-body-tertiary border-end p-3" id="doc-nav" tabindex="-1">
    <div class="d-flex d-lg-none justify-content-end">
      <button class="btn-close" type="button" data-bs-dismiss="offcanvas" data-bs-target="#doc-nav" aria-label="Close"></button>
    </div>
    <a class="navbar-brand fs-5 fw-bold d-block text-center mb-1" href="index.html">{brand}</a>
    <div class="text-center text-body-secondary small mb-3">v{version}</div>
    <ul class="nav nav-pills flex-column gap-1">
{nav}    </ul>
    {THEME_PICKER}  </nav>
  <main class="doc-main flex-grow-1 py-4 px-3 px-lg-4">
    <div class="doc-container container-xl">
{body}
    </div>
  </main>
</div>
<script src="{VENDOR_DIR}/npm/bootstrap@5.3.3/dist/js/bootstrap.bundle.min.js"></script>
<script>{REVEAL_JS}</script>
</body>
</html>
"##,
        title = escape(title),
        brand = escape(name),
        version = env!("CARGO_PKG_VERSION"),
        highlight_css = highlight::stylesheet(),
    )
}
