//! The small Markdown subset the documented types' doc comments are written in.
//!
//! Not a full Markdown implementation: it handles only what those comments
//! actually contain, and anything else is rendered as the text it is.

use maud::{html, Markup};

use crate::components::widget::{code_block, link, Callout};

/// Collapses each run of whitespace into a single space.
pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Renders the inline formatting `` `code` ``, `**bold**`, `*italic*`,
/// `[text](url)` links and bare `http(s)://` URLs, leaving all other text
/// escaped. A code span is marked `doc-code`.
pub fn inline(s: &str) -> Markup {
    inline_parts(s, true)
}

/// [`inline`], with bare URLs left as text when `linkify` is false.
fn inline_parts(s: &str, linkify: bool) -> Markup {
    let mut out: Vec<Markup> = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    // Plain text accumulates here and is flushed (escaped) on the next markup.
    let mut plain = String::new();
    macro_rules! flush {
        () => {{
            out.push(html! { (plain) });
            plain.clear();
        }};
    }
    while i < bytes.len() {
        let rest = &s[i..];
        if let Some(inner) = rest.strip_prefix('`') {
            if let Some(end) = inner.find('`') {
                flush!();
                out.push(html! { code class="doc-code" { (&inner[..end]) } });
                i += 1 + end + 1;
                continue;
            }
        }
        if let Some(inner) = rest.strip_prefix("**") {
            if let Some(end) = inner.find("**") {
                flush!();
                out.push(html! { strong { (inline_parts(&inner[..end], linkify)) } });
                i += 2 + end + 2;
                continue;
            }
        }
        if let Some(delim) = rest.chars().next().filter(|c| matches!(c, '*' | '_')) {
            let inner = &rest[1..];
            // An underscore within a word (`snake_case`) opens nothing, and a
            // closing one has to end a word rather than start one.
            let intraword = plain.chars().last().is_some_and(char::is_alphanumeric);
            let end = inner.find(delim).filter(|&end| end > 0);
            if let (false, Some(end)) = (intraword, end) {
                if !inner[end + 1..].chars().next().is_some_and(char::is_alphanumeric) {
                    flush!();
                    out.push(html! { em { (inline_parts(&inner[..end], linkify)) } });
                    i += 1 + end + 1;
                    continue;
                }
            }
        }
        if rest.starts_with('[') {
            if let Some((text, url, len)) = parse_link(rest) {
                flush!();
                // The text of a link cannot hold another one.
                out.push(link(url, inline_parts(text, false)));
                i += len;
                continue;
            }
        }
        if linkify {
            if let Some(url) = bare_url(rest) {
                flush!();
                out.push(link(url, html! { (url) }));
                i += url.len();
                continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        plain.push(ch);
        i += ch.len_utf8();
    }
    flush!();
    html! { @for part in out { (part) } }
}

/// The bare URL `s` starts with, taken as far as it runs. Punctuation that ends
/// a sentence rather than the address is left out, as is a closing bracket the
/// URL never opened. `None` unless `s` starts with a scheme followed by
/// something to address.
fn bare_url(s: &str) -> Option<&str> {
    let scheme = ["https://", "http://"].into_iter().find(|s2| s.starts_with(s2))?;
    let end = s.find([' ', '\t', '\n', '<', '`', '*', '"']).unwrap_or(s.len());
    let mut url = &s[..end];
    loop {
        let trimmed = url.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'']);
        // A URL may carry brackets, so one is only dropped when it closes
        // nothing inside the URL itself, as in "(see https://example.com/a)".
        let unopened = |open: char, close: char| {
            trimmed.ends_with(close)
                && trimmed.matches(open).count() < trimmed.matches(close).count()
        };
        let trimmed = match unopened('(', ')') || unopened('[', ']') {
            true => &trimmed[..trimmed.len() - 1],
            false => trimmed,
        };
        if trimmed.len() == url.len() {
            break;
        }
        url = trimmed;
    }
    (url.len() > scheme.len()).then_some(url)
}

fn parse_link(s: &str) -> Option<(&str, &str, usize)> {
    // Matching bracket rather than the first one: the link text may itself hold
    // a bracketed name, as it does for a link to a TOML table (`` [`[log]`] ``).
    let mut depth = 0usize;
    let close = s.char_indices().find_map(|(i, c)| match c {
        '[' => {
            depth += 1;
            None
        }
        ']' => {
            depth -= 1;
            (depth == 0).then_some(i)
        }
        _ => None,
    })?;
    let after = &s[close + 1..];
    if !after.starts_with('(') {
        return None;
    }
    let paren = after.find(')')?;
    let text = &s[1..close];
    let url = &after[1..paren];
    Some((text, url, close + 1 + paren + 1))
}

/// Renders paragraphs, ATX headings, `-`/`*` bullet lists, ```` ``` ```` fenced
/// code blocks (with an optional language for highlighting), and the inline
/// formatting handled by [`inline`]. Each heading is replaced by whatever
/// `heading` returns for it.
pub fn blocks(text: &str, heading: &mut dyn FnMut(usize, &Markup) -> Markup) -> Markup {
    let mut out: Vec<Markup> = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_end();

        if let Some(level) = heading_level(trimmed) {
            let text = trimmed.trim_start().trim_start_matches('#').trim_start();
            out.push(heading(level, &inline(text)));
            continue;
        }

        if let Some(lang) = trimmed.trim_start().strip_prefix("```") {
            let lang = lang.trim().to_string();
            let mut code = String::new();
            for body in lines.by_ref() {
                if body.trim_start().starts_with("```") {
                    break;
                }
                // With `trim = false` doc comments, each line keeps the single
                // conventional space after `///`. Dropping it puts the block's
                // own relative indentation at column zero.
                code.push_str(body.strip_prefix(' ').unwrap_or(body));
                code.push('\n');
            }
            out.push(code_block(&code, &lang));
            continue;
        }

        // Each bullet may wrap over following (non-blank, non-special) lines
        // until the next bullet or a blank line.
        if is_bullet(trimmed) {
            let mut items: Vec<Markup> = Vec::new();
            let mut item = bullet_text(trimmed).to_string();
            while let Some(next) = lines.peek() {
                let nt = next.trim_end();
                let nts = nt.trim_start();
                if nts.is_empty() || heading_level(nt).is_some() || nts.starts_with("```") {
                    break;
                }
                if is_bullet(nt) {
                    items.push(inline(&collapse_ws(&item)));
                    item = bullet_text(nt).to_string();
                } else {
                    item.push(' ');
                    item.push_str(nts);
                }
                lines.next();
            }
            items.push(inline(&collapse_ws(&item)));
            out.push(html! { ul { @for item in items { li { (item) } } } });
            continue;
        }

        if trimmed.is_empty() {
            continue;
        }

        let mut para = String::from(trimmed.trim_start());
        while let Some(next) = lines.peek() {
            let nt = next.trim_end();
            if nt.is_empty() || is_bullet(nt) || nt.trim_start().starts_with("```") {
                break;
            }
            para.push(' ');
            para.push_str(nt.trim_start());
            lines.next();
        }
        out.push(html! { p { (inline(&collapse_ws(&para))) } });
    }
    html! { @for part in out { (part) } }
}

/// Markdown from a doc comment: a heading naming a rustdoc section (e.g.
/// `# Warning` or `# Note`) is set, together with everything up to the next
/// heading, as a callout box. Every other heading is handed to `heading` as
/// [`blocks`] does.
pub fn doc_blocks(src: &str, heading: &mut dyn FnMut(usize, &Markup) -> Markup) -> Markup {
    let mut out: Vec<Markup> = Vec::new();
    let mut prose = String::new();
    let mut lines = src.lines().peekable();
    while let Some(line) = lines.next() {
        let callout = heading_text(line).and_then(|t| Callout::from_label(t));
        let Some(callout) = callout else {
            prose.push_str(line);
            prose.push('\n');
            continue;
        };
        out.push(blocks(&prose, &mut *heading));
        prose.clear();
        let mut section = String::new();
        while let Some(next) = lines.next_if(|l| heading_text(l).is_none()) {
            section.push_str(next);
            section.push('\n');
        }
        let inner = blocks(&section, &mut *heading);
        out.push(callout.to_markup(inner));
    }
    out.push(blocks(&prose, heading));
    html! { @for part in out { (part) } }
}

/// The source of the paragraph `src` opens with, and everything after it.
/// `None` when `src` opens with another kind of block, meaning a heading, a
/// bullet list, or a fenced code block.
pub fn split_paragraph(src: &str) -> Option<(&str, &str)> {
    let mut start: Option<usize> = None;
    let mut at = 0;
    for line in src.split_inclusive('\n') {
        let trimmed = line.trim();
        let breaks = trimmed.is_empty()
            || heading_level(trimmed).is_some()
            || is_bullet(trimmed)
            || trimmed.starts_with("```");
        match start {
            None if trimmed.is_empty() => {}
            None if breaks => return None,
            None => start = Some(at),
            Some(from) if breaks => return Some((&src[from..at], &src[at..])),
            Some(_) => {}
        }
        at += line.len();
    }
    start.map(|from| (&src[from..], ""))
}

/// The text of an ATX heading line (`Warning` for `# Warning`), or `None` if
/// the line is not one.
pub fn heading_text(line: &str) -> Option<&str> {
    let t = line.trim_end().trim_start();
    heading_level(t).map(|_| t.trim_start_matches('#').trim_start())
}

fn heading_level(line: &str) -> Option<usize> {
    let t = line.trim_start();
    let hashes = t.bytes().take_while(|&b| b == b'#').count();
    if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
        Some(hashes)
    } else {
        None
    }
}

fn is_bullet(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("- ") || t.starts_with("* ")
}

fn bullet_text(line: &str) -> &str {
    line.trim_start()[2..].trim_start()
}
