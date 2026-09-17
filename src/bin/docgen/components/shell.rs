//! The shared HTML shell every generated page is wrapped in: a left navigation
//! bar carrying the site brand, and the content area beside it.
//!
//! Styling uses the same Bootstrap version as the autograder's web UI (see
//! `web/templates`). The rules specific to these pages live in `docs.css`.

use maud::{html, Markup, PreEscaped};

use crate::components::body::Body;
use crate::highlight;

/// The relative directory (under the output dir) that the cached CDN assets are
/// written to, and referenced from in the generated pages.
pub const VENDOR_DIR: &str = "vendor";

/// Page-specific styling, inlined into every page's `<style>` block.
const DOC_CSS: &str = include_str!("../docs.css");

/// The colour-mode toggler the web UI uses (see `web/templates/header.stpl`),
/// inlined into `<head>` so the stored theme is applied before the first paint.
/// It drives the `data-bs-theme-value` buttons and the `#bd-theme` label that
/// [`theme_picker`] emits.
const COLOR_SCHEME_JS: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/web/static/script/color-scheme.js"));

/// Every symbol the pages reach with `<use href="#…">`, as an inline sprite:
/// the four the theme dropdown is built from, which the web UI inlines too, and
/// the callout, lock and copy-button ones.
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
  <!-- (https://icons.getbootstrap.com/icons/clipboard/) -->
  <symbol id="clipboard" viewBox="0 0 16 16"><path d="M4 1.5H3a2 2 0 0 0-2 2V14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V3.5a2 2 0 0 0-2-2h-1v1h1a1 1 0 0 1 1 1V14a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V3.5a1 1 0 0 1 1-1h1v-1z"></path><path d="M9.5 1a.5.5 0 0 1 .5.5v1a.5.5 0 0 1-.5.5h-3a.5.5 0 0 1-.5-.5v-1a.5.5 0 0 1 .5-.5h3zm-3-1A1.5 1.5 0 0 0 5 1.5v1A1.5 1.5 0 0 0 6.5 4h3A1.5 1.5 0 0 0 11 2.5v-1A1.5 1.5 0 0 0 9.5 0h-3z"></path></symbol>
</svg>
"##;

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

/// Copies the code a [`super::widget::code_block`] button belongs to, marking
/// the button for a moment so the click is acknowledged. One delegated listener
/// covers every block on the page, including those revealed later.
const COPY_JS: &str = r##"(() => {
  "use strict";
  document.addEventListener("click", (ev) => {
    const button = ev.target.closest(".doc-copy");
    if (!button) return;
    const code = button.closest(".doc-code-wrap").querySelector("code");
    navigator.clipboard.writeText(code.textContent).then(() => {
      button.querySelector("use").setAttribute("href", "#check2");
      setTimeout(() => button.querySelector("use").setAttribute("href", "#clipboard"), 1500);
    }, () => {});
  });
})();
"##;

/// The Light/Dark/Auto dropdown, driven by [`COLOR_SCHEME_JS`]. It opens
/// upwards, sitting at the bottom of the sidebar.
fn theme_picker() -> Markup {
    let modes = [
        ("light", "sun-fill", "Light", false),
        ("dark", "moon-stars-fill", "Dark", false),
        ("auto", "circle-half", "Auto", true),
    ];
    html! {
        div class="dropup doc-theme mt-auto pt-2 border-top" {
            button id="bd-theme" class="doc-theme-btn btn btn-link nav-link dropdown-toggle d-flex w-100 align-items-center justify-content-center" type="button" data-bs-toggle="dropdown" data-bs-display="static" aria-expanded="false" aria-label="Toggle theme (auto)" {
                svg class="bi theme-icon-active" aria-hidden="true" { use href="#circle-half" {} }
                span class="ms-2" id="bd-theme-text" { "Toggle theme" }
            }
            ul class="doc-theme-menu dropdown-menu" aria-labelledby="bd-theme-text" {
                @for (value, icon, label, current) in modes {
                    @let class = match current {
                        true => "dropdown-item d-flex align-items-center active",
                        false => "dropdown-item d-flex align-items-center",
                    };
                    li {
                        button type="button" class=(class) data-bs-theme-value=(value)
                            aria-pressed=(if current { "true" } else { "false" }) {
                            svg class="bi me-2 opacity-50" aria-hidden="true" {
                                use href={ "#" (icon) } {}
                            }
                            (label) " "
                            svg class="bi ms-auto d-none" aria-hidden="true" {
                                use href="#check2" {}
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Wraps a page body in the shared site shell: the Bootstrap left sidebar and
/// the content area. `name` is the brand at the top, `title` heads the browser
/// tab, and `nav` gives the sidebar entries as `(file name, label)` with the one
/// at `active` highlighted. The entries the body recorded are listed as links
/// under the active entry. A body that recorded none gets no submenu.
pub fn html_page(
    name: &str,
    title: &str,
    nav: &[(&str, &str)],
    active: usize,
    body: Body,
) -> Markup {
    let submenu = body.submenu();
    let body = body.into_html();

    // maud's own `DOCTYPE` is upper-case, and the site has always emitted the
    // lower-case spelling.
    html! {
        (PreEscaped("<!doctype html>"))
        html lang="en" data-bs-theme="light" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " - " (name) " Documentation" }
                link href={ (VENDOR_DIR) "/npm/bootstrap@5.3.3/dist/css/bootstrap.min.css" }
                    rel="stylesheet";
                script { (PreEscaped(COLOR_SCHEME_JS)) }
                style {
                    "\n" (PreEscaped(DOC_CSS))
                    "\n" (PreEscaped(highlight::stylesheet())) "\n"
                }
            }
            body {
                (PreEscaped(ICONS))
                nav class="navbar bg-body-tertiary border-bottom sticky-top d-lg-none" {
                    div class="container-fluid" {
                        button class="navbar-toggler" type="button" data-bs-toggle="offcanvas" data-bs-target="#doc-nav" aria-controls="doc-nav" aria-label="Toggle navigation" {
                            span class="navbar-toggler-icon" {}
                        }
                        a class="navbar-brand fw-bold ms-3 me-auto" href="index.html" { (name) }
                    }
                }
                div class="d-flex" {
                    nav class="doc-sidebar offcanvas-lg offcanvas-start bg-body-tertiary border-end p-3" id="doc-nav" tabindex="-1" {
                        div class="d-flex d-lg-none justify-content-end" {
                            button class="btn-close" type="button" data-bs-dismiss="offcanvas" data-bs-target="#doc-nav" aria-label="Close" {}
                        }
                        a class="navbar-brand fs-5 fw-bold d-block text-center mb-1" href="index.html" {
                            (name)
                        }
                        div class="text-center text-body-secondary small mb-3" {
                            "v" (env!("CARGO_PKG_VERSION"))
                        }
                        ul class="nav nav-pills flex-column gap-1" {
                            @for (i, (file, label)) in nav.iter().enumerate() {
                                @let active = i == active;
                                li class="nav-item" {
                                    a class=(if active { "nav-link active" } else { "nav-link" })
                                        href=(file) { (label) }
                                    @if active { (submenu) }
                                }
                            }
                        }
                        (theme_picker())
                    }
                    main class="doc-main flex-grow-1 py-4 px-3 px-lg-4" {
                        div class="doc-container container-xl" { (body) }
                    }
                }
                script src={ (VENDOR_DIR) "/npm/bootstrap@5.3.3/dist/js/bootstrap.bundle.min.js" } {}
                script { (PreEscaped(REVEAL_JS)) }
                script { (PreEscaped(COPY_JS)) }
            }
        }
    }
}

/// Warns on stderr about every fragment `html` points at that no `id` in the
/// same document carries. A fragment counts whether it comes from a link, a
/// `data-bs-target` or the sprite's `<use href="#icon">`. `file` names the page
/// in the warning.
pub fn warn_dangling_fragments(file: &str, html: &str) {
    // Each attribute is matched with its leading space, so a name merely ending
    // in `id` cannot pass as one.
    let values = |attr: &str| -> Vec<&str> {
        html.match_indices(attr)
            .filter_map(|(at, _)| html[at + attr.len()..].split('"').next())
            .collect()
    };
    let ids = values(" id=\"");
    for attr in [" href=\"#", " data-bs-target=\"#"] {
        for fragment in values(attr) {
            if !ids.contains(&fragment) {
                eprintln!(
                    "warning: {file}: link to #{fragment}, which nothing on the page carries"
                );
            }
        }
    }
}
