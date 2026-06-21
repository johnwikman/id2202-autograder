use actix_web::{error::InternalError, http::StatusCode, HttpResponse, HttpResponseBuilder};
use id2202_autograder::config::Settings;
use sailfish::TemplateSimple;

use crate::route::common::CommonInformation;

/// Renders a 401 unauthorized page
pub fn unauthorized(settings: &Settings) -> Result<HttpResponse, actix_web::Error> {
    UnauthorizedMessageTemplate::unauthorized(settings, None)
}

/// Renders a 404 not found page
pub fn not_found(settings: &Settings) -> Result<HttpResponse, actix_web::Error> {
    ErrorMessageTemplate::not_found(settings)
}

/// Renders a 500 internal server error page
pub fn internal_server_error(settings: &Settings) -> Result<HttpResponse, actix_web::Error> {
    ErrorMessageTemplate::internal_server_error(settings)
}

#[derive(TemplateSimple)]
#[template(path = "error/message.stpl")]
struct ErrorMessageTemplate {
    common: CommonInformation,
    // Message to show on the page
    msg: String,
}

impl ErrorMessageTemplate {
    fn not_found(settings: &Settings) -> Result<HttpResponse, actix_web::Error> {
        ErrorMessageTemplate {
            common: CommonInformation::from_title(settings, "404: Not Found"),
            msg: String::from("404: Not Found \u{2639}"),
        }
        .render_errmsg_template(HttpResponse::NotFound())
    }
    /// Renders a 500 not found page
    fn internal_server_error(settings: &Settings) -> Result<HttpResponse, actix_web::Error> {
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

#[derive(TemplateSimple)]
#[template(path = "error/unauthorized.stpl")]
pub struct UnauthorizedMessageTemplate<'a> {
    common: CommonInformation,
    reason: Option<&'a str>,
}

impl<'a> UnauthorizedMessageTemplate<'a> {
    /// Renders a 401 unauthorized page
    fn unauthorized(
        settings: &Settings,
        reason: Option<&'a str>,
    ) -> Result<HttpResponse, actix_web::Error> {
        UnauthorizedMessageTemplate {
            common: CommonInformation::from_title(settings, "401: Unauthorized"),
            reason: reason,
        }
        .render_errmsg_template(HttpResponse::Unauthorized())
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
