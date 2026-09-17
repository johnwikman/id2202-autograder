//! Generates the static HTML documentation site (settings, test configuration,
//! and the REST API).

use std::path::Path;

use clap::Parser;

use id2202_autograder::{config::Settings, error::Error};

mod components;
mod highlight;
mod markdown;
mod openapi;
mod route;
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

    /// Path to the OpenAPI JSON produced by `server emit-openapi`. The REST API
    /// page is rendered from it.
    #[arg(long)]
    openapi: String,
}

fn main() -> Result<(), Error> {
    let args = Args::parse();
    let cfg = Settings::load(&args.settings)?;

    let dir = Path::new(&args.out);
    std::fs::create_dir_all(dir).map_err(|e| {
        Error::fs("creating docs output directory", &args.out).with_cause(Box::new(e))
    })?;

    let raw = std::fs::read_to_string(&args.openapi)
        .map_err(|e| Error::fs("reading openapi spec", &args.openapi).with_cause(Box::new(e)))?;
    let spec: openapi::Spec = serde_json::from_str(&raw)
        .map_err(|e| Error::runtime(format!("could not parse OpenAPI spec: {e}")))?;

    let options = route::RenderOptions { name: &cfg.name, spec: &spec };
    for (page, contents) in route::render_all(&options) {
        let contents = contents.into_string();
        components::warn_dangling_fragments(page, &contents);
        let path = dir.join(page);
        std::fs::write(&path, contents).map_err(|e| {
            Error::fs("writing docs page", path.to_string_lossy()).with_cause(Box::new(e))
        })?;
        println!("wrote {}", path.display());
    }

    // The generated site is fully self-contained and needs no CDN at view time.
    for file in cdn_cache::CDN_FILES {
        let path = dir.join(components::VENDOR_DIR).join(file.served_as);
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
