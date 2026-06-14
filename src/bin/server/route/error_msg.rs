use actix_web::{error::InternalError, http::StatusCode, HttpResponse, HttpResponseBuilder};
use id2202_autograder::config::Settings;
use sailfish::TemplateSimple;

use crate::route::common::CommonInformation;

#[derive(TemplateSimple)]
#[template(path = "error/message.stpl")]
pub struct ErrorMessageTemplate {
    common: CommonInformation,
    // Message to show on the page
    msg: String,
}

impl ErrorMessageTemplate {
    /// Renders a 404 unauthorized page
    pub fn unauthorized(settings: &Settings) -> Result<HttpResponse, actix_web::Error> {
        ErrorMessageTemplate {
            common: CommonInformation::from_title(settings, "401: Unauthorized"),
            msg: String::from("401: Unauthorized \u{274C}"),
        }
        .render_errmsg_template(HttpResponse::Unauthorized())
    }
    /// Renders a 404 not found page
    pub fn not_found(settings: &Settings) -> Result<HttpResponse, actix_web::Error> {
        ErrorMessageTemplate {
            common: CommonInformation::from_title(settings, "404: Not Found"),
            msg: String::from("404: Not Found \u{2639}"),
        }
        .render_errmsg_template(HttpResponse::NotFound())
    }
    /// Renders a 500 not found page
    pub fn internal_server_error(settings: &Settings) -> Result<HttpResponse, actix_web::Error> {
        ErrorMessageTemplate {
            common: CommonInformation::from_title(settings, "500: Internal Server Error"),
            msg: String::from("500: Internal Server Error"),
        }
        .render_errmsg_template(HttpResponse::InternalServerError())
    }

    /// Internal rendering function
    fn render_errmsg_template(
        self,
        mut resp: HttpResponseBuilder,
    ) -> Result<HttpResponse, actix_web::Error> {
        let body: String = self
            .render_once()
            .map_err(|e| InternalError::new(e, StatusCode::INTERNAL_SERVER_ERROR))?;

        Ok(resp.body(body))
    }
}
