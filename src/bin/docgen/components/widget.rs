//! The small Bootstrap widgets the pages are built from.

use maud::{html, Markup, PreEscaped};

use crate::highlight;

/// A lock glyph, drawn from the sprite [`super::shell`] inlines.
pub const LOCK_ICON: PreEscaped<&str> = PreEscaped(
    r##"<svg class="bi doc-icon-inline" aria-hidden="true"><use href="#lock-fill"></use></svg>"##,
);

/// A bordered box whose heading is a badge sitting on its top-left border.
pub fn notched_box(title: Markup, content: Markup) -> Markup {
    html! {
        div class="doc-box border rounded-3 p-3 pt-4 mt-3 mb-3 position-relative bg-secondary-subtle" {
            span class="position-absolute top-0 start-0 translate-middle-y ms-3 badge text-bg-secondary" {
                (title)
            }
            (content)
        }
    }
}

/// A disclosure block, framed as a card (see `.doc-details` in `docs.css`).
/// `summary` is the clickable header bar. `content` is hidden until it is
/// opened.
pub fn details(summary: &str, content: Markup) -> Markup {
    html! {
        details class="doc-details mb-3" {
            summary class="doc-summary" { (summary) }
            (content)
        }
    }
}

/// A link. An address that leaves the documentation opens in a tab of its own,
/// meaning one carrying a scheme (`https:`, `mailto:`), one beginning `//`, and
/// an absolute path. A relative path and a fragment stay in the current tab.
pub fn link(href: &str, content: Markup) -> Markup {
    let scheme = href.split_once(':').is_some_and(|(scheme, _)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    let leaves = scheme || href.starts_with('/');
    html! {
        a href=(href) target=[leaves.then_some("_blank")]
            rel=[leaves.then_some("noopener noreferrer")] {
            (content)
        }
    }
}

/// A code block with a copy button. `code` is highlighted as `lang`, or left as
/// escaped plain text when `lang` names no known syntax.
pub fn code_block(code: &str, lang: &str) -> Markup {
    // The button is a sibling of the `<pre>` rather than a child: a `<pre>`
    // scrolls, and a button inside one scrolls out of view with the code.
    html! {
        div class="doc-code-wrap" {
            pre class="doc-code-block" { code { (PreEscaped(highlight::highlight(code, lang))) } }
            button type="button" class="doc-copy" aria-label="Copy" {
                svg class="bi" aria-hidden="true" { use href="#clipboard" {} }
            }
        }
    }
}

/// A Bootstrap-styled table whose every column but the last is marked
/// `doc-cell-narrow`, leaving the last one the remaining width.
pub fn html_table(headers: &[&str], rows: &[Vec<Markup>]) -> Markup {
    let narrow = |i: usize, len: usize| (i + 1 < len).then_some("doc-cell-narrow");
    html! {
        div class="table-responsive" {
            table class="table doc-table" {
                thead {
                    tr {
                        @for (i, h) in headers.iter().enumerate() {
                            th class=[narrow(i, headers.len())] { (h) }
                        }
                    }
                }
                tbody {
                    @for row in rows {
                        tr {
                            @for (i, cell) in row.iter().enumerate() {
                                td class=[narrow(i, row.len())] { (cell) }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A callout box (see `.doc-callout` in `docs.css`): a tinted bar carrying
/// `icon` and `label`, with `content` below it.
pub fn callout(label: &str, class: &str, icon: &str, content: Markup) -> Markup {
    html! {
        div class={ "doc-callout " (class) } {
            div class="doc-callout-label" {
                svg class="bi" aria-hidden="true" { use href={ "#" (icon) } {} }
                (label)
            }
            div class="doc-callout-body" { (content) }
        }
    }
}
