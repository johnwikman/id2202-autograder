//! Generates the static HTML documentation site (settings, test configuration,
//! and — when `--openapi` is given — the REST API).
//!
//! This is a separate binary, built only under the `docs` feature, so its
//! syntax-highlighting dependencies stay out of the shipped binaries. See
//! `just gen-docs`.

use std::path::Path;

use clap::Parser;

use id2202_autograder::{config::Settings, error::Error};

mod highlight;
mod html;
mod markdown;
mod page;
mod schema;

/// Locally cached third-party web assets (Bootstrap), embedded at build time.
/// See `web/cdncache/` and `build.rs`. Written next to the docs under `vendor/`.
pub(crate) mod cdn_cache {
    include!(concat!(env!("OUT_DIR"), "/generated_cdn_cache.rs"));
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the TOML file containing the program settings. Used to read the
    /// autograder `name`, which becomes the site brand and title.
    #[arg(short, long)]
    settings: String,

    /// Output directory for the generated HTML pages.
    #[arg(short, long, default_value = "target/docs/site")]
    out: String,

    /// Path to the OpenAPI JSON produced by `server emit-openapi`. When given,
    /// the REST API page is rendered from it.
    #[arg(long)]
    openapi: Option<String>,
}

/// Writes the documentation site into `out_dir`. `name` (from the settings file)
/// is used as the site brand and title. When `openapi_path` is given, the REST
/// API page is rendered from that OpenAPI JSON.
fn write_site(name: &str, out_dir: &str, openapi_path: Option<&str>) -> Result<(), Error> {
    let dir = Path::new(out_dir);
    std::fs::create_dir_all(dir).map_err(|e| {
        Error::fs("creating docs output directory", out_dir).with_cause(Box::new(e))
    })?;

    let mut pages = vec![
        ("index.html", page::index::render(name)),
        ("settings.html", page::settings::render(name)),
        ("tests.html", page::test_configuration::render(name)),
    ];

    if let Some(spec_path) = openapi_path {
        let raw = std::fs::read_to_string(spec_path)
            .map_err(|e| Error::fs("reading openapi spec", spec_path).with_cause(Box::new(e)))?;
        let spec: page::rest_api::Spec = serde_json::from_str(&raw)
            .map_err(|e| Error::runtime(format!("could not parse OpenAPI spec: {e}")))?;
        pages.push(("api.html", page::rest_api::render(name, &spec)));
    }

    for (page, contents) in pages {
        let path = dir.join(page);
        std::fs::write(&path, contents).map_err(|e| {
            Error::fs("writing docs page", path.to_string_lossy()).with_cause(Box::new(e))
        })?;
        println!("wrote {}", path.display());
    }

    // Write the cached CDN assets (Bootstrap) into `vendor/` so the generated
    // site is fully self-contained and needs no CDN at view time.
    for file in cdn_cache::CDN_FILES {
        let path = dir.join(html::VENDOR_DIR).join(file.served_as);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Error::fs("creating vendor directory", parent.to_string_lossy())
                    .with_cause(Box::new(e))
            })?;
        }
        std::fs::write(&path, file.bytes).map_err(|e| {
            Error::fs("writing vendor asset", path.to_string_lossy()).with_cause(Box::new(e))
        })?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn main() -> Result<(), Error> {
    let args = Args::parse();
    let cfg = Settings::load(&args.settings)?;
    write_site(&cfg.name, &args.out, args.openapi.as_deref())
}
