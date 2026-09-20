//! Static syntax highlighting for the generated documentation.
//!
//! Highlighting happens at generation time, not in the browser: there is no
//! JavaScript component. [`highlight`] turns a code block into `<span>`s
//! carrying `hl-`-prefixed CSS classes, and [`stylesheet`] emits the matching
//! CSS once into the page's `<style>` block.

use std::sync::OnceLock;

use syntect::highlighting::ThemeSet;
use syntect::html::{css_for_theme_with_class_style, ClassStyle, ClassedHTMLGenerator};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use two_face::theme::EmbeddedThemeName;

// The `hl-` prefix keeps the generated classes clear of Bootstrap's own.
const CLASS_STYLE: ClassStyle = ClassStyle::SpacedPrefixed { prefix: "hl-" };

/// A theme of syntect's own bundled set, named as [`ThemeSet::load_defaults`]
/// keys it.
const LIGHT_THEME: &str = "InspiredGitHub";

const DARK_THEME: EmbeddedThemeName = EmbeddedThemeName::ColdarkDark;

/// Prefixes every selector in a syntect stylesheet with `scope`.
fn scoped(css: &str, scope: &str) -> String {
    // syntect emits one selector list per line, ending in `{`, with the
    // properties on the lines that follow.
    let mut out = String::with_capacity(css.len());
    for line in css.lines() {
        let selectors = line.trim().strip_suffix('{').filter(|s| s.contains('.'));
        match selectors {
            Some(selectors) => {
                let scoped: Vec<String> =
                    selectors.split(',').map(|s| format!("{scope} {}", s.trim())).collect();
                out.push_str(&format!("{} {{\n", scoped.join(", ")));
            }
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

/// The CSS rules for the highlighting classes, one palette per colour mode.
/// The scopes exclude one another because a light rule naming more classes than
/// its dark counterpart would otherwise win in dark mode.
pub fn stylesheet() -> String {
    let themes = ThemeSet::load_defaults();
    let light = css_for_theme_with_class_style(&themes.themes[LIGHT_THEME], CLASS_STYLE)
        .unwrap_or_default();
    let extra = two_face::theme::extra();
    let dark =
        css_for_theme_with_class_style(extra.get(DARK_THEME), CLASS_STYLE).unwrap_or_default();
    // Bootstrap's colour-mode attribute, set on `<html>` by `color-scheme.js`.
    let light = scoped(&light, ":root:not([data-bs-theme=\"dark\"])");
    let dark = scoped(&dark, ":root[data-bs-theme=\"dark\"]");
    format!("{light}\n{dark}")
}

/// Highlights `code` as `lang` (e.g. `"json"`, `"toml"`), returning HTML with
/// `hl-`-prefixed spans. An unknown or empty `lang` falls back to plain,
/// escaped text.
pub fn highlight(code: &str, lang: &str) -> String {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    // syntect's bundled syntax set has JSON but not TOML, so the syntaxes come
    // from `two-face`, which ships the assets `bat` does and includes both.
    let ss = SYNTAXES.get_or_init(two_face::syntax::extra_newlines);
    let syntax = ss.find_syntax_by_token(lang).unwrap_or_else(|| ss.find_syntax_plain_text());
    let mut generator = ClassedHTMLGenerator::new_with_class_style(syntax, ss, CLASS_STYLE);
    for line in LinesWithEndings::from(code) {
        // Only fails on malformed syntax definitions, which the bundled ones
        // are not. Ignoring it keeps highlighting from aborting generation.
        let _ = generator.parse_html_for_line_which_includes_newline(line);
    }
    generator.finalize()
}
