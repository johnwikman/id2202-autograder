//! The tiny Markdown subset used in the crate's doc comments.
//!
//! Deliberately not a full Markdown implementation: it handles only what the
//! doc comments actually contain, so that user-facing prose can live on the
//! types it documents rather than being hard-coded in the renderer.

use crate::html::code_block;

/// Collapses each run of whitespace into a single space.
pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Escapes the five characters that are significant in HTML text/attributes.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Renders inline formatting — `` `code` ``, `**bold**`, and `[text](url)`
/// links — leaving all other text escaped.
pub fn inline(s: &str) -> String {
    let mut out = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    // Plain text accumulates here and is flushed (escaped) on the next markup.
    let mut plain = String::new();
    macro_rules! flush {
        () => {{
            out.push_str(&escape(&plain));
            plain.clear();
        }};
    }
    while i < bytes.len() {
        let rest = &s[i..];
        if let Some(inner) = rest.strip_prefix('`') {
            if let Some(end) = inner.find('`') {
                flush!();
                out.push_str("<code>");
                out.push_str(&escape(&inner[..end]));
                out.push_str("</code>");
                i += 1 + end + 1;
                continue;
            }
        }
        if let Some(inner) = rest.strip_prefix("**") {
            if let Some(end) = inner.find("**") {
                flush!();
                out.push_str("<strong>");
                out.push_str(&inline(&inner[..end]));
                out.push_str("</strong>");
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
                if !inner[end + 1..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphanumeric)
                {
                    flush!();
                    out.push_str("<em>");
                    out.push_str(&inline(&inner[..end]));
                    out.push_str("</em>");
                    i += 1 + end + 1;
                    continue;
                }
            }
        }
        if rest.starts_with('[') {
            if let Some((text, url, len)) = parse_link(rest) {
                flush!();
                out.push_str(&format!("<a href=\"{}\">{}</a>", escape(url), inline(text)));
                i += len;
                continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        plain.push(ch);
        i += ch.len_utf8();
    }
    flush!();
    out
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
/// formatting handled by [`inline`].
///
/// Headings are handed to `heading` (as their level and already-rendered inner
/// HTML) rather than written out here, so the caller can give them ids and
/// record them: a heading written in a doc comment reaches the sidebar submenu
/// the same way one written by a page does.
pub fn markdown(text: &str, heading: &mut dyn FnMut(usize, &str) -> String) -> String {
    let mut out = String::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_end();

        if let Some(level) = heading_level(trimmed) {
            let text = trimmed.trim_start().trim_start_matches('#').trim_start();
            out.push_str(&heading(level, &inline(text)));
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
                // conventional space after `///`; drop it so the block's own
                // relative indentation starts at column zero.
                code.push_str(body.strip_prefix(' ').unwrap_or(body));
                code.push('\n');
            }
            out.push_str(&code_block(&code, &lang));
            continue;
        }

        // Each bullet may wrap over following (non-blank, non-special) lines
        // until the next bullet or a blank line.
        if is_bullet(trimmed) {
            out.push_str("<ul>\n");
            let mut item = bullet_text(trimmed).to_string();
            while let Some(next) = lines.peek() {
                let nt = next.trim_end();
                let nts = nt.trim_start();
                if nts.is_empty() || heading_level(nt).is_some() || nts.starts_with("```") {
                    break;
                }
                if is_bullet(nt) {
                    out.push_str(&format!("<li>{}</li>\n", inline(&collapse_ws(&item))));
                    item = bullet_text(nt).to_string();
                } else {
                    item.push(' ');
                    item.push_str(nts);
                }
                lines.next();
            }
            out.push_str(&format!("<li>{}</li>\n", inline(&collapse_ws(&item))));
            out.push_str("</ul>\n");
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
        out.push_str(&format!("<p>{}</p>\n", inline(&collapse_ws(&para))));
    }
    out
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
