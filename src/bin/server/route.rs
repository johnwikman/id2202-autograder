use actix_web::{
    web::{self, ServiceConfig},
    HttpResponse,
};
use actix_web_static_files::ResourceFiles;

use id2202_autograder::config::Settings;

mod common;
mod error_msg;
mod index;
mod job_info;
mod submission;

// Used for generated static routes below
// (See build.rs in root of the repository.)
mod static_route_generator {
    include!(concat!(env!("OUT_DIR"), "/generated_web_static.rs"));
}

pub fn config(cfg: &mut ServiceConfig, _settings: &Settings) {
    cfg.route("/", web::get().to(|data| index::get_index("/", data)));

    cfg.route(
        "/job_info",
        web::get().to(|data, req| job_info::get_job_info("/job_info", data, req)),
    );

    cfg.service(web::resource("/submission/{id}").route(web::get().to(submission::get_submission)));
    cfg.service(
        web::resource("/submission/{id}/").route(web::get().to(submission::get_submission)),
    );
    cfg.service(
        web::resource("/submission/{id}/markdown")
            .route(web::get().to(submission::get_submission_markdown)),
    );

    // Setup static routes
    let generated = static_route_generator::generate();
    cfg.service(ResourceFiles::new("/static", generated));

    // Favicon should point to a static resource
    cfg.service(web::redirect("/favicon.ico", "/static/image/favicon.ico"));
}

/// Default page to use if not found
/// this function is configured in main.rs
pub fn not_found(settings: &Settings) -> Result<HttpResponse, actix_web::Error> {
    error_msg::not_found(settings)
}
