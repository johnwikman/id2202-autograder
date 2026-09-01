//! Functionality for expression structured text in a flexible format, such
//! that it can be rendered on a web page, markdown, or other formats.

use std::fmt::{self, Write};

use crate::config::ReportingSettings;

/// One or more structured paragraphs that abstractly carries its structure, so
/// that the same text can be written as Markdown or HTML.
#[derive(Debug, Clone)]
pub enum StructuredParagraph<'a> {
    Many(Vec<StructuredParagraph<'a>>),
    Paragraph(StructuredInline<'a>),
    Itemized(Vec<StructuredInline<'a>>),
}

#[derive(Debug, Clone)]
pub enum StructuredInline<'a> {
    Sep {
        sep: &'a str,
        parts: Vec<StructuredInline<'a>>,
    },
    Sentences(Vec<StructuredInline<'a>>),
    OxfordCommaSepWords(Vec<StructuredInline<'a>>),
    InlineCode(Box<StructuredInline<'a>>),
    Bold(Box<StructuredInline<'a>>),
    /// Plain string. This will escape characters depending on the backend.
    Plain(String),
    PlainStr(&'a str),
    /// A string that should we written exactly as it is shown.
    Raw(String),
    RawStr(&'a str),
}

impl<'a> StructuredParagraph<'a> {
    pub fn render_markdown(
        &self,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result {
        RendererMarkdown::render_paragraph(self, settings, dst)
    }

    pub fn render_html(&self, settings: &ReportingSettings, dst: &mut impl Write) -> fmt::Result {
        RendererHTML::render_paragraph(self, settings, dst)
    }

    pub fn plain(s: String) -> Self {
        StructuredParagraph::Paragraph(StructuredInline::plain(s))
    }

    pub fn plain_str(s: &'a str) -> Self {
        StructuredParagraph::Paragraph(StructuredInline::plain_str(s))
    }

    pub fn inline_code(content: impl Into<StructuredInline<'a>>) -> Self {
        StructuredParagraph::Paragraph(StructuredInline::inline_code(content))
    }

    pub fn bold(content: impl Into<StructuredInline<'a>>) -> Self {
        StructuredParagraph::Paragraph(StructuredInline::bold(content))
    }
}

impl<'a> From<&'a str> for StructuredParagraph<'a> {
    fn from(value: &'a str) -> Self {
        StructuredParagraph::plain_str(value)
    }
}

impl<'a> From<String> for StructuredParagraph<'a> {
    fn from(value: String) -> Self {
        StructuredParagraph::plain(value)
    }
}

impl<'a> StructuredInline<'a> {
    pub fn plain(s: String) -> Self {
        StructuredInline::Plain(s)
    }

    pub fn plain_str(s: &'a str) -> Self {
        StructuredInline::PlainStr(s)
    }

    pub fn inline_code(content: impl Into<StructuredInline<'a>>) -> Self {
        StructuredInline::InlineCode(Box::new(content.into()))
    }

    pub fn bold(content: impl Into<StructuredInline<'a>>) -> Self {
        StructuredInline::Bold(Box::new(content.into()))
    }

    /// Separates the parts with the string " ": a single space.
    pub fn sep_space(parts: Vec<StructuredInline<'a>>) -> Self {
        StructuredInline::Sep { sep: " ", parts }
    }

    /// Separates the parts with the string ", ": a comma followed by a space.
    pub fn sep_comma(parts: Vec<StructuredInline<'a>>) -> Self {
        StructuredInline::Sep { sep: ", ", parts }
    }
}

impl<'a> From<&'a str> for StructuredInline<'a> {
    fn from(value: &'a str) -> Self {
        StructuredInline::plain_str(value)
    }
}

impl<'a> From<String> for StructuredInline<'a> {
    fn from(value: String) -> Self {
        StructuredInline::plain(value)
    }
}

pub trait TextRenderer {
    /// Entrypoint for rendering paragraphs
    fn render_paragraph(
        par: &StructuredParagraph,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result {
        match par {
            StructuredParagraph::Paragraph(p) => Self::r_paragraph(p, settings, dst),
            StructuredParagraph::Itemized(items) => Self::r_itemized(items, settings, dst),
            StructuredParagraph::Many(pars) => Self::r_many(pars, settings, dst),
        }
    }

    /// Entrypoint for rendering (inline) text
    fn render_text(
        txt: &StructuredInline,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result {
        match txt {
            StructuredInline::Plain(s) => Self::r_plain(s, settings, dst),
            StructuredInline::PlainStr(s) => Self::r_plain(s, settings, dst),
            StructuredInline::Raw(s) => dst.write_str(s),
            StructuredInline::RawStr(s) => dst.write_str(s),
            StructuredInline::InlineCode(c) => Self::r_inlinecode(c, settings, dst),
            StructuredInline::Bold(c) => Self::r_bold(c, settings, dst),
            StructuredInline::Sep { sep, parts } => Self::r_sep(sep, parts, settings, dst),
            StructuredInline::Sentences(sentences) => Self::r_sentences(sentences, settings, dst),
            StructuredInline::OxfordCommaSepWords(words) => {
                Self::r_oxfordcommasep(words, settings, dst)
            }
        }
    }

    // StructuedParagraph functionality
    fn r_paragraph(
        txt: &StructuredInline,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result;
    fn r_itemized(
        items: &[StructuredInline],
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result;
    fn r_many(
        pars: &[StructuredParagraph],
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result;

    // StructuredText functionality
    fn r_plain(txt: &str, settings: &ReportingSettings, dst: &mut impl Write) -> fmt::Result;
    fn r_inlinecode<'a>(
        code: &StructuredInline<'a>,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result;
    fn r_bold<'a>(
        content: &StructuredInline<'a>,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result;

    /// Writes parts to dst, interleaving them with the provided separator.
    fn r_sep(
        sep: &str,
        parts: &[StructuredInline],
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result {
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                dst.write_str(sep)?;
            }
            Self::render_text(part, settings, dst)?;
        }
        Ok(())
    }

    /// Writes sentences to dst, interleaving them with spaces and ending sentence with a dot.
    fn r_sentences(
        sentences: &[StructuredInline],
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result {
        for (i, sentence) in sentences.iter().enumerate() {
            if i > 0 {
                dst.write_char(' ')?;
            }
            Self::render_text(sentence, settings, dst)?;
            dst.write_char('.')?;
        }
        Ok(())
    }

    /// Writes oxford comma separated words.
    fn r_oxfordcommasep(
        words: &[StructuredInline],
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> Result<(), std::fmt::Error> {
        match words {
            [] => Ok(()),
            [word] => Self::render_text(word, settings, dst),
            [w1, w2] => {
                Self::render_text(w1, settings, dst)?;
                dst.write_str(" and ")?;
                Self::render_text(w2, settings, dst)
            }
            [initial @ .., last] => {
                for word in initial {
                    Self::render_text(word, settings, dst)?;
                    dst.write_str(", ")?;
                }
                dst.write_str("and ")?;
                Self::render_text(last, settings, dst)
            }
        }
    }
}

struct RendererMarkdown;

impl TextRenderer for RendererMarkdown {
    fn r_paragraph(
        txt: &StructuredInline,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result {
        Self::render_text(txt, settings, dst)
    }

    fn r_itemized(
        items: &[StructuredInline],
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result {
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                dst.write_str("\n\n")?;
            }
            // TODO: Now we assume that the item here is not a paragraph...
            dst.write_str(" * ")?;
            Self::render_text(item, settings, dst)?;
        }
        Ok(())
    }

    fn r_many(
        pars: &[StructuredParagraph],
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result {
        for (i, par) in pars.iter().enumerate() {
            if i > 0 {
                dst.write_str("\n\n")?;
            }
            Self::render_paragraph(par, settings, dst)?;
        }
        Ok(())
    }

    /// Escapes markdown characters within the string `txt`.
    ///
    /// This escapes the following characters by putting a backslash `\` in front of them:
    ///
    /// ```txt
    /// \ ` * _ { } [ ] ( ) # + - . !
    /// ```
    ///
    /// See https://www.markdownlang.com/basic/escaping.html
    fn r_plain(txt: &str, _settings: &ReportingSettings, dst: &mut impl Write) -> fmt::Result {
        const ESC_CHARS: &str = "\\`*_{}[]()#+-.!";
        for ch in txt.chars() {
            if ESC_CHARS.contains(ch) {
                dst.write_char('\\')?;
            }
            dst.write_char(ch)?;
        }
        Ok(())
    }

    fn r_inlinecode<'a>(
        code: &StructuredInline<'a>,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result {
        dst.write_char('`')?;
        Self::render_text(code, settings, dst)?;
        dst.write_char('`')?;
        Ok(())
    }

    fn r_bold<'a>(
        content: &StructuredInline<'a>,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result {
        dst.write_str("**")?;
        Self::render_text(content, settings, dst)?;
        dst.write_str("**")?;
        Ok(())
    }
}

struct RendererHTML;

impl TextRenderer for RendererHTML {
    fn r_paragraph(
        txt: &StructuredInline,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result {
        dst.write_str("<p>")?;
        Self::render_text(txt, settings, dst)?;
        dst.write_str("</p>")
    }

    fn r_itemized(
        items: &[StructuredInline],
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result {
        dst.write_str("<ul>")?;
        for item in items {
            dst.write_str("<li>")?;
            Self::render_text(item, settings, dst)?;
            dst.write_str("</li>")?;
        }
        dst.write_str("</ul>")
    }

    fn r_many(
        pars: &[StructuredParagraph],
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result {
        for par in pars {
            Self::render_paragraph(par, settings, dst)?;
        }
        Ok(())
    }

    /// Escapes HTML characters within the string `txt`.
    fn r_plain(txt: &str, _settings: &ReportingSettings, dst: &mut impl Write) -> fmt::Result {
        write!(dst, "{}", v_htmlescape::escape_fmt(txt))?;
        Ok(())
    }

    fn r_inlinecode<'a>(
        code: &StructuredInline<'a>,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result {
        dst.write_str("<code>")?;
        Self::render_text(code, settings, dst)?;
        dst.write_str("</code>")?;
        Ok(())
    }

    fn r_bold<'a>(
        content: &StructuredInline<'a>,
        settings: &ReportingSettings,
        dst: &mut impl Write,
    ) -> fmt::Result {
        dst.write_str("<b>")?;
        Self::render_text(content, settings, dst)?;
        dst.write_str("</b>")?;
        Ok(())
    }
}
