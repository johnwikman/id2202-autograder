//! The verifier tools reference: the `autograder_verifier_tools` package as
//! `griffe dump` describes it.
//!
//! A symbol is documented by its signature rather than by its name, and every
//! type in a signature links to the entry defining it.

use std::collections::BTreeMap;

use maud::{html, Markup};

use crate::components::widget::Callout;
use crate::components::{html_table, slug, value_markdown, warn_untyped, Anchor, Body};
use crate::griffe::{source_text, Dump, Kind, Object};
use crate::markdown::{collapse_ws, inline, split_paragraph};

/// Every documented name, and the anchor its entry carries. Minted before
/// anything is rendered, so a signature can link to a type declared below it.
type Index<'a> = BTreeMap<&'a str, Anchor>;

const CONTEXT: &str = "verifier tools";

pub fn body(dump: &Dump) -> Body {
    let mut body = Body::new(html! { h1 { "Autograder Verifier Tools Reference" } });
    let Some(package) = dump.values().next() else {
        eprintln!("warning: {CONTEXT}: the griffe dump describes no package");
        return body;
    };
    if dump.len() > 1 {
        eprintln!("warning: {CONTEXT}: {} more packages in the dump are not shown", dump.len() - 1);
    }

    let exports = package.exports();
    let index: Index = exports
        .iter()
        .map(|object| (object.name.as_str(), body.anchor(&entry_slug(object))))
        .collect();

    body.markdown(package.docs());
    body.push(note());

    body.heading(2, html! { "Overview" });
    body.push(overview(&exports, &index));

    let (functions, types): (Vec<&Object>, Vec<&Object>) =
        exports.iter().copied().partition(|object| object.kind == Kind::Function);
    for (title, section) in [("Classes", types), ("Functions", functions)] {
        if section.is_empty() {
            continue;
        }
        body.heading(2, html! { (title) });
        for object in section {
            render(&mut body, object, &index);
        }
    }
    body
}

/// A function keeps its parentheses, so that a link to it reads as a call.
fn entry_slug(object: &Object) -> String {
    match object.kind {
        Kind::Function => slug(&format!("{}()", object.name)),
        _ => slug(&object.name),
    }
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Module => "module",
        Kind::Class => "class",
        Kind::Function => "function",
        Kind::Attribute => "attribute",
        Kind::Alias | Kind::Other => "",
    }
}

/// The first paragraph, which is where the convention puts the one-line
/// description.
fn summary(object: &Object) -> Markup {
    match split_paragraph(object.docs()) {
        Some((para, _)) => inline(&collapse_ws(para)),
        None => Markup::default(),
    }
}

/// The one thing on the page not read out of the package. Its link to
/// `run_verifier` is unchecked, since `warn_dangling_fragments` only looks
/// within a page.
fn note() -> Markup {
    let inner = html! {
        p {
            "Using this package is recommended, not required. It is located under "
            "the example verifier container image ("
            code class="doc-code" { "example/container/verifier" }
            "). A verifier that does not use this library has to implement the JSON-based "
            "verifier protocol itself. This protocol is described under the "
            a href="tests.html#run-verifier" { "run_verifier" } " test kind documentation."
        }
    };
    Callout::from_label("note").expect("`note` is a callout label").to_markup(inner)
}

fn overview(exports: &[&Object], index: &Index) -> Markup {
    let rows: Vec<Vec<Markup>> = exports
        .iter()
        .map(|object| {
            let anchor = &index[object.name.as_str()];
            vec![
                html! { a href=(anchor.href()) { code class="doc-field" { (object.name) } } },
                html! { span class="badge text-bg-secondary" { (kind_label(object.kind)) } },
                summary(object),
            ]
        })
        .collect();
    html_table(&["Name", "Kind", "Summary"], &rows)
}

/// A top-level symbol is given a heading, so that it carries the same weight
/// as a class rather than reading as one of its members.
fn render(body: &mut Body, object: &Object, index: &Index) {
    let anchor = index[object.name.as_str()].clone();
    let label = html! {
        code class="doc-name" {
            (object.name)
            @if object.kind == Kind::Function { "()" }
        }
    };
    body.sig_heading(3, &anchor, signature(object, None, index), label);
    body.markdown(object.docs());
    if object.kind == Kind::Class {
        members(body, object, &anchor, index);
    }
}

fn members(body: &mut Body, class: &Object, anchor: &Anchor, index: &Index) {
    let fields = class.members_of(Kind::Attribute);
    if !fields.is_empty() {
        body.push(attribute_table(&fields, index));
    }

    // Each method records a submenu entry as it is built, so the rail is
    // assembled first and pushed whole.
    let mut rail: Vec<Markup> = Vec::new();
    for method in class.members_of(Kind::Function) {
        let member = body.anchor(&format!("{}--{}", anchor.id(), entry_slug(method)));
        body.nested_entry(anchor, &member, html! { code class="doc-name" { (method.name) "()" } });
        rail.push(definition(&member, method, Some(&class.name), index));
    }
    if !rail.is_empty() {
        body.push(html! {
            div class="doc-members" {
                p class="doc-members-label" { "Methods" }
                @for method in rail { (method) }
            }
        });
    }
}

fn definition(anchor: &Anchor, object: &Object, owner: Option<&str>, index: &Index) -> Markup {
    html! {
        dl class="doc-dl" {
            dt class="doc-entry" id=(anchor.id()) {
                (signature(object, owner, index))
                a class="doc-anchor" href=(anchor.href()) aria-label="Link to this section" { "#" }
            }
            dd class="doc-entry-body" { (value_markdown(object.docs())) }
        }
    }
}

fn attribute_table(fields: &[&Object], index: &Index) -> Markup {
    let mut untyped: Vec<String> = Vec::new();
    let rows: Vec<Vec<Markup>> = fields
        .iter()
        .map(|field| {
            let badge = field.annotation.as_ref().map(|a| annotation_badge(&source_text(a), index));
            if badge.is_none() {
                untyped.push(field.name.clone());
            }
            vec![
                html! { code class="doc-field" { (field.name) } },
                badge.unwrap_or_default(),
                value_markdown(field.docs()),
            ]
        })
        .collect();
    warn_untyped(CONTEXT, &untyped);
    html_table(&["Attribute", "Type", "Description"], &rows)
}

/// Tinted as the configuration pages tint theirs.
fn annotation_badge(annotation: &str, index: &Index) -> Markup {
    let scalar = |name: &str| match name {
        "str" => "ty-string",
        "int" => "ty-integer",
        "bool" => "ty-boolean",
        _ => "ty-other",
    };
    let class = match annotation.split_once('[') {
        Some(("list", element)) => format!("{} is-array", scalar(element.trim_end_matches(']'))),
        _ => scalar(annotation).to_string(),
    };
    html! { span class={ "badge setting-type " (class) } { (linked(annotation, index)) } }
}

/// The dim prefixes before a name, as in `class Encoded` or
/// `contextmanager no_except`.
fn roles(object: &Object) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // A decorator such as `@dataclass` arrives twice, once as the label griffe
    // derived from it and once as the decorator itself.
    let mut push = |role: String| {
        if !out.contains(&role) {
            out.push(role);
        }
    };
    match object.kind {
        Kind::Attribute => push("attribute".to_string()),
        Kind::Class => {
            let exception = object.bases.iter().any(|base| source_text(base) == "Exception");
            push(match exception {
                true => "exception".to_string(),
                false => "class".to_string(),
            });
            for label in &object.labels {
                push(label.clone());
            }
        }
        _ => {}
    }
    for decorator in &object.decorators {
        let call = source_text(&decorator.value);
        push(match call.split_once('(') {
            Some((name, _)) => name.to_string(),
            None => call,
        });
    }
    out
}

/// The line an entry is titled by.
fn signature(object: &Object, owner: Option<&str>, index: &Index) -> Markup {
    html! {
        code class="doc-sig" {
            @for role in roles(object) { span class="doc-sig-role" { (role) } }
            @if let Some(owner) = owner { span class="doc-sig-owner" { (owner) "." } }
            span class="doc-sig-name" { (object.name) }
            @if object.kind == Kind::Function {
                span class="doc-sig-punct" { "(" }
                @let taken = object
                    .parameters
                    .iter()
                    .filter(|p| !matches!(p.name.as_str(), "self" | "cls"));
                @for (i, param) in taken.enumerate() {
                    @if i > 0 { ", " }
                    span class="doc-sig-param" { (param.name) }
                    @if let Some(annotation) = &param.annotation {
                        ": " (linked(&source_text(annotation), index))
                    }
                    @if let Some(default) = &param.default { " = " (source_text(default)) }
                }
                span class="doc-sig-punct" { ")" }
                @if let Some(returns) = &object.returns {
                    span class="doc-sig-arrow" { " \u{2192} " }
                    (linked(&source_text(returns), index))
                }
            }
            @if object.kind == Kind::Attribute {
                @if let Some(annotation) = &object.annotation {
                    span class="doc-sig-arrow" { ": " }
                    (linked(&source_text(annotation), index))
                } @else if let Some(value) = &object.value {
                    span class="doc-sig-arrow" { " = " }
                    (linked(&source_text(value), index))
                }
            }
        }
    }
}

/// An annotation with every documented name linked, so that
/// `dict[str, Encoded]` reaches the entry for `Encoded`.
fn linked(annotation: &str, index: &Index) -> Markup {
    let identifier = |c: char| c.is_alphanumeric() || c == '_';
    let mut parts: Vec<Markup> = Vec::new();
    let mut rest = annotation;
    while !rest.is_empty() {
        let split = match rest.starts_with(identifier) {
            true => rest.find(|c: char| !identifier(c)),
            false => rest.find(identifier),
        };
        let (head, tail) = rest.split_at(split.unwrap_or(rest.len()));
        parts.push(match index.get(head) {
            Some(anchor) => html! { a class="doc-sig-type" href=(anchor.href()) { (head) } },
            None => html! { (head) },
        });
        rest = tail;
    }
    html! { @for part in parts { (part) } }
}
