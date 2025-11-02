use actix_web::{web, App, HttpRequest, HttpServer, Responder};
use id2202_autograder::{error::Error, settings::Settings};

use clap::Parser;

mod api;
mod route;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the TOML file containing the program settings
    #[arg(short, long)]
    settings: String,
}

async fn not_found(req: HttpRequest) -> Result<impl Responder, actix_web::Error> {
    if req.path().starts_with("/api") {
        api::not_found(req)
    } else {
        route::not_found()
    }
}

#[actix_web::main]
async fn main() -> Result<(), Error> {
    let args: Args = Args::parse();
    let s = Settings::load(&args.settings)?;
    s.setup_logging("server")?;
    let s_clone1 = s.clone();
    HttpServer::new(move || {
        let s = s_clone1.clone();
        App::new()
            .configure(|cfg| route::config(cfg, &s))
            .configure(|cfg| api::config(cfg, &s))
            .default_service(web::to(not_found))
    })
    .bind((s.server.address, s.server.port))?
    .run()
    .await
    .map_err(Error::from)
}
