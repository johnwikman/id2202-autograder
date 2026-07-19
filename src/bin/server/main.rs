use actix_web::{web, App, HttpServer};
use id2202_autograder::{config::Settings, error::Error};

use clap::{Parser, Subcommand};

mod api;
mod auth;
mod openapi;
mod route;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the TOML file containing the program settings
    #[arg(short, long)]
    settings: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the HTTP server
    Serve,

    /// Write the OpenAPI specification (JSON) to a file and exit, without
    /// starting the server.
    EmitOpenapi {
        /// Path to write the OpenAPI JSON to.
        output: String,
    },
}

#[actix_web::main]
async fn main() -> Result<(), Error> {
    let args: Args = Args::parse();

    match &args.command {
        Command::Serve => serve(&args.settings).await,
        Command::EmitOpenapi { output } => emit_openapi(output),
    }
}

async fn serve(settings: &str) -> Result<(), Error> {
    use actix_web::middleware::Logger;

    let s = Settings::load(settings)?;
    s.setup_logging("server")?;
    let s_clone1 = s.clone();
    HttpServer::new(move || {
        let s = s_clone1.clone();
        App::new()
            .wrap(Logger::default())
            .wrap(actix_web::middleware::from_fn(auth::authenticate))
            .wrap(actix_web::middleware::NormalizePath::trim())
            .app_data(web::Data::new(s.clone()))
            .configure(|cfg| route::config(cfg, &s))
            .service(
                web::scope("/api")
                    .wrap(actix_web::middleware::from_fn(async |req, next| {
                        api::auth_hook("/api", req, next).await
                    }))
                    .configure(|cfg| api::config(cfg, &s)),
            )
            .default_service(web::to(async |data: web::Data<Settings>| {
                route::not_found(data.get_ref())
            }))
    })
    .bind((s.server.address, s.server.port))?
    .run()
    .await
    .map_err(Error::from)
}

fn emit_openapi(path: &str) -> Result<(), Error> {
    if let Some(parent) = std::path::Path::new(path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|e| {
            Error::fs(
                "creating openapi output directory",
                parent.to_string_lossy(),
            )
            .with_cause(Box::new(e))
        })?;
    }
    std::fs::write(path, openapi::spec_json()?)
        .map_err(|e| Error::fs("writing openapi spec", path).with_cause(Box::new(e)))?;
    println!("wrote {path}");
    Ok(())
}
