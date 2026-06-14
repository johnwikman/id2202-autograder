use actix_web::{
    error::InternalError,
    http::StatusCode,
    web::{self},
    HttpResponse, Responder,
};
use sailfish::TemplateSimple;

use id2202_autograder::config::Settings;

use crate::route::common::CommonInformation;

/// Index (Home) page
#[derive(TemplateSimple)]
#[template(path = "route/index.stpl")]
struct IndexTemplate {
    common: CommonInformation,
}

/// The home page.
pub async fn get_index(
    current_route: &str,
    data: web::Data<Settings>,
) -> Result<impl Responder, actix_web::Error> {
    let settings = data.get_ref();

    let tpl = IndexTemplate {
        common: CommonInformation::from_title_route(settings, "Home", current_route),
    };

    let body: String = tpl
        .render_once()
        .map_err(|e| InternalError::new(e, StatusCode::INTERNAL_SERVER_ERROR))?;

    Ok(HttpResponse::Ok().body(body))
}
