//! Static syntax highlighting for the generated documentation.
//!
//! Highlighting is done at generation time, not in the browser: there is no
//! JavaScript component. Each code block is turned into `<span>`s carrying
//! `hl-`-prefixed CSS classes, and [`stylesheet`] emits the matching CSS once
//! into the page's `<style>` block. Because the markup carries classes rather
//! than colours, both colour modes are served by the same HTML: [`stylesheet`]
//! emits a light palette and a dark one scoped under `[data-bs-theme="dark"]`.
//!
//! syntect's bundled syntax set has JSON but not TOML, so the syntaxes come from
//! `two-face` (the same syntax assets `bat` ships), which includes both.

use std::sync::OnceLock;

use syntect::highlighting::ThemeSet;
use syntect::html::{css_for_theme_with_class_style, ClassStyle, ClassedHTMLGenerator};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use two_face::theme::EmbeddedThemeName;

/// Prefix applied to every generated CSS class, so the highlighting styles never
/// collide with Bootstrap's own classes.
const CLASS_STYLE: ClassStyle = ClassStyle::SpacedPrefixed { prefix: "hl-" };

/// syntect's bundled light theme; its palette suits the Bootstrap light theme.
const THEME: &str = "InspiredGitHub";

/// The palette used in dark mode, from `two-face`'s extra themes.
const DARK_THEME: EmbeddedThemeName = EmbeddedThemeName::ColdarkDark;

/// Selector the dark palette is scoped under: Bootstrap's colour-mode attribute,
/// set on `<html>` by `color-scheme.js`.
const DARK_SCOPE: &str = "[data-bs-theme=\"dark\"]";

/// The light rules stay in force in dark mode wherever the dark theme sets no
/// rule of its own, so emphasis leaks across (InspiredGitHub italicises
/// comments, Coldark-Dark does not). This resets it just above the light rules'
/// specificity and just below the dark ones', and is emitted before them.
const DARK_RESET: &str =
    "[data-bs-theme=\"dark\"] [class*=\"hl-\"] { font-style: normal; font-weight: normal; }\n";

fn syntaxes() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(two_face::syntax::extra_newlines)
}

/// Prefixes every rule in a syntect stylesheet with `scope`, so it only applies
/// inside a matching element. syntect emits one selector list per line, ending
/// in `{`, with the properties on the lines that follow.
fn scoped(css: &str, scope: &str) -> String {
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

/// The CSS rules for the highlighting classes, to be inlined into the page
/// `<style>` block. Both palettes are emitted: [`THEME`] unconditionally, and
/// [`DARK_THEME`] scoped to dark mode, where it overrides the light rules.
pub fn stylesheet() -> String {
    let themes = ThemeSet::load_defaults();
    let light =
        css_for_theme_with_class_style(&themes.themes[THEME], CLASS_STYLE).unwrap_or_default();
    let extra = two_face::theme::extra();
    let dark =
        css_for_theme_with_class_style(extra.get(DARK_THEME), CLASS_STYLE).unwrap_or_default();
    format!("{light}\n{DARK_RESET}{}", scoped(&dark, DARK_SCOPE))
}

/// Highlights `code` as `lang` (e.g. `"json"`, `"toml"`), returning HTML with
/// `hl-`-prefixed spans. Unknown languages (or an empty `lang`) fall back to
/// plain, escaped text, so this is always safe to call.
pub fn highlight(code: &str, lang: &str) -> String {
    let ss = syntaxes();
    let syntax = ss.find_syntax_by_token(lang).unwrap_or_else(|| ss.find_syntax_plain_text());
    let mut generator = ClassedHTMLGenerator::new_with_class_style(syntax, ss, CLASS_STYLE);
    for line in LinesWithEndings::from(code) {
        // Only fails on malformed syntax definitions, which the bundled ones are
        // not; ignore so highlighting never aborts generation.
        let _ = generator.parse_html_for_line_which_includes_newline(line);
    }
    generator.finalize()
}
