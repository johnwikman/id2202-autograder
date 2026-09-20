//! The page body under construction, the ids it mints for the elements written
//! through it, and [`slug`], which turns a label into such an id.

use maud::{html, Markup};

use crate::markdown::doc_blocks;

/// The id of an element of the page, unique within it. Obtainable only from the
/// [`Body`] that minted it.
#[derive(Clone)]
pub struct Anchor(String);

impl Anchor {
    /// The bare id, for the element's own `id="…"`.
    pub fn id(&self) -> &str {
        &self.0
    }

    /// The id as a fragment reference, `#id`.
    pub fn href(&self) -> String {
        format!("#{}", self.0)
    }
}

/// An entry in the sidebar submenu, recorded as the body is written.
struct Heading {
    /// Each level past 2 is indented a further step in the submenu. Zero for
    /// an entry indented by its nesting instead.
    level: usize,
    anchor: Anchor,
    /// Inner HTML, reused as the submenu label.
    label: Markup,
    /// Hidden behind a caret in the submenu.
    children: Vec<Heading>,
}

/// A page body under construction. A heading written through it is given an id,
/// a link to itself, and an entry in the sidebar submenu.
pub struct Body {
    html: Vec<Markup>,
    toc: Vec<Heading>,
    /// Every id minted so far, headings and [`Body::anchor`] alike.
    ids: Vec<String>,
}

impl Body {
    pub fn new(html: Markup) -> Self {
        Body { html: vec![html], toc: Vec::new(), ids: Vec::new() }
    }

    /// Any heading in `html` stays out of the submenu.
    pub fn push(&mut self, html: Markup) {
        self.html.push(html);
    }

    pub fn heading(&mut self, level: usize, content: Markup) -> Anchor {
        let (anchor, html) = self.heading_html(level, &content, None);
        self.html.push(html);
        anchor
    }

    /// A heading that is nothing but the name of a thing, such as a TOML table
    /// or a test kind, which `docs.css` sets apart from a heading that only
    /// mentions one.
    pub fn name_heading(&mut self, level: usize, name: &str) -> Anchor {
        let inner = html! { code class="doc-name" { (name) } };
        let (anchor, html) = self.heading_html(level, &inner, Some("doc-heading-name"));
        self.html.push(html);
        anchor
    }

    /// Markdown, whose own headings are treated as [`Body::heading`] does,
    /// bar the rustdoc sections [`doc_blocks`] boxes.
    pub fn markdown(&mut self, src: &str) {
        let html = doc_blocks(src, &mut |level, inner| self.heading_html(level, inner, None).1);
        self.html.push(html);
    }

    /// An id based on `base`, reserved against every later mint but emitted
    /// nowhere: the caller writes the element carrying it.
    pub fn anchor(&mut self, base: &str) -> Anchor {
        self.mint(base.to_string())
    }

    /// A heading for an id [`Body::anchor`] minted earlier, whose content is
    /// the signature of what it documents. The submenu lists it as `label`,
    /// since a whole signature makes an unreadable entry there.
    pub fn sig_heading(&mut self, level: usize, anchor: &Anchor, sig: Markup, label: Markup) {
        let html = self.heading_markup(level, anchor, &sig, Some("doc-heading-sig"));
        self.html.push(html);
        self.toc.push(Heading { level, anchor: anchor.clone(), label, children: Vec::new() });
    }

    /// A submenu entry for something that is not a heading.
    pub fn entry(&mut self, level: usize, anchor: &Anchor, label: Markup) {
        self.toc.push(Heading { level, anchor: anchor.clone(), label, children: Vec::new() });
    }

    /// A submenu entry listed under the heading `parent` records. Falls back
    /// to the top level when `parent` is not a heading of its own, since the
    /// lookup does not descend.
    pub fn nested_entry(&mut self, parent: &Anchor, anchor: &Anchor, label: Markup) {
        let entry = Heading { level: 0, anchor: anchor.clone(), label, children: Vec::new() };
        match self.toc.iter_mut().find(|h| h.anchor.id() == parent.id()) {
            Some(parent) => parent.children.push(entry),
            None => self.toc.push(entry),
        }
    }

    /// The `<ul>` linking to everything recorded, for the sidebar. Empty when
    /// nothing was recorded.
    pub fn submenu(&self) -> Markup {
        if self.toc.is_empty() {
            return Markup::default();
        }
        html! { ul class="nav flex-column doc-toc ms-3" { (submenu_items(&self.toc)) } }
    }

    pub fn into_html(self) -> Markup {
        html! { @for part in self.html { (part) } }
    }

    /// `base`, made unique against every id minted before it and recorded as
    /// taken. An empty `base` is replaced by a positional name.
    fn mint(&mut self, base: String) -> Anchor {
        let mut id = match base.is_empty() {
            true => format!("section-{}", self.toc.len() + 1),
            false => base,
        };
        while self.ids.contains(&id) {
            id.push('-');
        }
        self.ids.push(id.clone());
        Anchor(id)
    }

    /// The heading markup, with a unique id, the heading classes plus
    /// `extra_class`, and a link to itself, recording the submenu entry on the
    /// way.
    fn heading_html(
        &mut self,
        level: usize,
        content: &Markup,
        extra_class: Option<&str>,
    ) -> (Anchor, Markup) {
        let anchor = self.mint(slug(&content.0));
        let out = self.heading_markup(level, &anchor, content, extra_class);
        let entry =
            Heading { level, anchor: anchor.clone(), label: content.clone(), children: Vec::new() };
        self.toc.push(entry);
        (anchor, out)
    }

    /// One heading, carrying `anchor` as its id and a link to itself.
    fn heading_markup(
        &self,
        level: usize,
        anchor: &Anchor,
        content: &Markup,
        extra_class: Option<&str>,
    ) -> Markup {
        let id = anchor.id();
        let class = match extra_class {
            Some(extra) => format!("doc-heading doc-h{level} {extra}"),
            None => format!("doc-heading doc-h{level}"),
        };
        let inner = html! {
            (content)
            a class="doc-anchor" href=(anchor.href()) aria-label="Link to this section" { "#" }
        };
        // An element name has to be a literal in `html!`, so each level is spelt
        // out rather than interpolated.
        match level {
            2 => html! { h2 id=(id) class=(class) { (inner) } },
            3 => html! { h3 id=(id) class=(class) { (inner) } },
            4 => html! { h4 id=(id) class=(class) { (inner) } },
            5 => html! { h5 id=(id) class=(class) { (inner) } },
            6 => html! { h6 id=(id) class=(class) { (inner) } },
            _ => html! { h1 id=(id) class=(class) { (inner) } },
        }
    }
}

/// An entry with children becomes a caret that reveals them.
fn submenu_items(headings: &[Heading]) -> Markup {
    html! {
        @for h in headings {
            @let indent = match h.level > 2 {
                true => format!(" lvl-{}", h.level),
                false => String::new(),
            };
            @let link = html! {
                a class={ "nav-link py-1" (indent) } href=(h.anchor.href()) { (h.label) }
            };
            @if h.children.is_empty() {
                li class="nav-item" { (link) }
            } @else {
                @let panel = format!("toc-{}", h.anchor.id());
                li class="nav-item doc-toc-group" {
                    div class="d-flex align-items-center" {
                        a class="doc-toc-caret nav-link py-1" data-bs-toggle="collapse"
                            href={ "#" (panel) } role="button" aria-expanded="false"
                            aria-controls=(panel) aria-label="Show members" {}
                        (link)
                    }
                    ul class="nav flex-column collapse doc-toc-sub" id=(panel) {
                        (submenu_items(&h.children))
                    }
                }
            }
        }
    }
}

/// Turns a label, which may be a heading's inner HTML, a TOML table name or a
/// schema title, into an anchor id: tags dropped, lowercased, and every run of
/// non-alphanumeric characters collapsed to a single dash.
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
