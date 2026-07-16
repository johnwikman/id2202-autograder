use actix_web::{web, App, HttpServer};
use id2202_autograder::{config::Settings, error::Error};

use clap::Parser;

mod api;
mod auth;
mod route;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the TOML file containing the program settings
    #[arg(short, long)]
    settings: String,
}

#[actix_web::main]
async fn main() -> Result<(), Error> {
    use actix_web::middleware::Logger;

    let args: Args = Args::parse();
    let s = Settings::load(&args.settings)?;
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
                    .wrap(actix_web::middleware::from_fn(async |req, next| api::auth_hook("/api", req, next).await))
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
