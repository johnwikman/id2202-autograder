use actix_web::{http::StatusCode, HttpRequest, HttpResponse};
use serde::Serialize;

use derive_more::derive::{Display, Error};
use num_traits::FromPrimitive;

use id2202_autograder::db::models::SubmissionWithReport;

#[derive(Serialize, Debug, Display, Error)]
#[display("error response: {status} on {path}")]
pub struct ErrorResponse {
    status: u16,
    error: String,
    message: String,
    path: String,
    method: String,
}

impl actix_web::error::ResponseError for ErrorResponse {
    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code()).json(self)
    }
    fn status_code(&self) -> actix_web::http::StatusCode {
        StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

impl ErrorResponse {
    pub fn unauthorized(req: &HttpRequest, msg: &str) -> ErrorResponse {
        ErrorResponse {
            status: StatusCode::UNAUTHORIZED.as_u16(),
            error: "unauthorized".to_string(),
            message: msg.to_string(),
            path: req.path().to_string(),
            method: req.method().to_string(),
        }
    }
    pub fn bad_request(req: &HttpRequest, msg: &str) -> ErrorResponse {
        ErrorResponse {
            status: StatusCode::BAD_REQUEST.as_u16(),
            error: "bad request".to_string(),
            message: msg.to_string(),
            path: req.path().to_string(),
            method: req.method().to_string(),
        }
    }
    pub fn not_found(req: &HttpRequest, msg: &str) -> ErrorResponse {
        ErrorResponse {
            status: StatusCode::NOT_FOUND.as_u16(),
            error: "not found".to_string(),
            message: msg.to_string(),
            path: req.path().to_string(),
            method: req.method().to_string(),
        }
    }
    pub fn internal_server_error(req: &HttpRequest) -> ErrorResponse {
        ErrorResponse {
            status: StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
            error: "internal server error".to_string(),
            message: "contact autograder responsible".to_string(),
            path: req.path().to_string(),
            method: req.method().to_string(),
        }
    }
}

/// Response to send back upon a submission.
#[derive(Debug, Serialize)]
pub struct SubmitResponse {
    code: u16,
    message: String,
    path: String,
}

impl SubmitResponse {
    pub fn new(req: &HttpRequest, msg: &str) -> SubmitResponse {
        SubmitResponse {
            code: StatusCode::OK.as_u16(),
            message: msg.to_string(),
            path: req.path().to_string(),
        }
    }
    pub fn to_http(&self) -> HttpResponse {
        HttpResponse::Ok().json(self)
    }
}

/// Information about a submission, to be sent back upon successful request.
#[derive(Debug, Serialize)]
pub struct SubmissionResponse<'a> {
    code: u16,
    path: String,
    finished: bool,
    successful: Option<bool>,
    submission: &'a SubmissionWithReport,
}

impl<'a> SubmissionResponse<'a> {
    pub fn new(req: &HttpRequest, sub: &'a SubmissionWithReport) -> SubmissionResponse<'a> {
        use id2202_autograder::db::models::SubmissionStatusCode as SSC;

        SubmissionResponse {
            code: StatusCode::OK.as_u16(),
            path: req.path().to_string(),
            finished: sub.exec_finished,
            successful: if sub.exec_finished {
                SSC::from_i32(sub.exec_status_code).map(|c| c == SSC::Success)
            } else {
                None
            },
            submission: sub,
        }
    }
    pub fn to_http(&self) -> HttpResponse {
        HttpResponse::Ok().json(self)
    }
}
