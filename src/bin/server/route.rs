use actix_web::{
    error::InternalError,
    get,
    http::StatusCode,
    web::{self, ServiceConfig},
    HttpRequest, HttpResponse, HttpResponseBuilder, Responder,
};
use actix_web_static_files::ResourceFiles;
use num_traits::FromPrimitive;
use sailfish::TemplateSimple;

use id2202_autograder::{config::Settings, reporting::Report, utils::systemtime_to_utc_string};
use serde::Deserialize;

// Used for generated static routes below
// (See build.rs in root of the repository.)
mod static_route_generator {
    include!(concat!(env!("OUT_DIR"), "/generated_web_static.rs"));
}

// SAILFISH_ variables are used inside the templates
static SAILFISH_HEADER_BAR_ROUTES: [(&str, &str); 2] = [("/", "Home"), ("/job_info", "Job Info")];
static SAILFISH_HEADER_BAR_TITLE: &str = "ID2202 Autograder";
static SAILFISH_TITLE_PREFIX: &str = "ID2202 | ";

pub fn config(cfg: &mut ServiceConfig, settings: &Settings) {
    cfg.service(get_index);

    let s = settings.clone();
    cfg.route(
        "/job_info",
        web::get().to(move |req| get_job_info(s.clone(), req)),
    );

    let s = settings.clone();
    cfg.service(
        web::resource("/submission/{id}")
            .route(web::get().to(move |req, id| get_submission(s.clone(), req, id))),
    );

    // Setup static routes
    let generated = static_route_generator::generate();
    cfg.service(ResourceFiles::new("/static", generated));
}

/// Default page to use if not found
#[derive(TemplateSimple)]
#[template(path = "error/message.stpl")]
struct ErrorMessageTemplate {
    // Common
    title: String,
    current_route: Option<String>,
    // Message to show on the page
    msg: String,
}

impl ErrorMessageTemplate {
    /// Renders a 404 unauthorized page
    pub fn unauthorized() -> Result<HttpResponse, actix_web::Error> {
        ErrorMessageTemplate {
            title: String::from("401: Unauthorized"),
            current_route: None,
            msg: String::from("401: Unauthorized \u{274C}"),
        }
        .render_errmsg_template(HttpResponse::Unauthorized())
    }
    /// Renders a 404 not found page
    pub fn not_found() -> Result<HttpResponse, actix_web::Error> {
        ErrorMessageTemplate {
            title: String::from("404: Not Found"),
            current_route: None,
            msg: String::from("404: Not Found \u{2639}"),
        }
        .render_errmsg_template(HttpResponse::NotFound())
    }
    /// Renders a 500 not found page
    pub fn internal_server_error() -> Result<HttpResponse, actix_web::Error> {
        ErrorMessageTemplate {
            title: String::from("500: Internal Server Error"),
            current_route: None,
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

/// this function is configured in main.rs
pub fn not_found() -> Result<HttpResponse, actix_web::Error> {
    ErrorMessageTemplate::not_found()
}

/// Index (Home) page
#[derive(TemplateSimple)]
#[template(path = "route/index.stpl")]
struct IndexTemplate {
    // Common
    title: String,
    current_route: Option<String>,
}

/// The home page.
///
/// This has no fancy behavior, so we use the top-level get("/") routing.
#[get("/")]
async fn get_index() -> Result<impl Responder, actix_web::Error> {
    let tpl = IndexTemplate {
        title: format!("Home"),
        current_route: Some(String::from("/")),
    };
    let body: String = tpl
        .render_once()
        .map_err(|e| InternalError::new(e, StatusCode::INTERNAL_SERVER_ERROR))?;

    Ok(HttpResponse::Ok().body(body))
}

/// Template for showing job information
#[derive(TemplateSimple)]
#[template(path = "route/job_info.stpl")]
struct JobInfoTemplate {
    // Common
    title: String,
    current_route: Option<String>,
    // For this template only
    job_info_vec: Vec<JobInfo>,
}

struct JobInfo {
    id: i64,
    date_submitted: String,
    grading_tags: Vec<String>,
    status: String,
    /// ("symbol", "span class")
    status_symbol_and_class: Option<(String, String)>,
}

/// Page that shows information about current jobs.
async fn get_job_info(
    settings: Settings,
    _req: HttpRequest,
) -> Result<HttpResponse, actix_web::Error> {
    use id2202_autograder::db::{
        conn::DatabaseConnection, models::Submission, models::SubmissionStatusCode as SSC,
    };

    let mut conn = match DatabaseConnection::connect(&settings) {
        Ok(conn) => conn,
        Err(e) => {
            log::error!("Could not open database connection: {e}");
            return ErrorMessageTemplate::internal_server_error();
        }
    };

    // Shows the last 100 submissions
    let subs: Vec<Submission> = {
        use diesel::{self, ExpressionMethods, QueryDsl, RunQueryDsl, SelectableHelper};
        use id2202_autograder::db::schema::submissions::{self, id};
        match submissions::table
            .select(Submission::as_select())
            .order(id.desc())
            .limit(100)
            .load(&mut conn.conn)
        {
            Ok(v) => v,
            Err(e) => {
                log::error!("Could not get submissions from database: {e}");
                return ErrorMessageTemplate::internal_server_error();
            }
        }
    };

    let tpl = JobInfoTemplate {
        title: format!("Job Info"),
        current_route: Some(format!("/job_info")),
        job_info_vec: subs
            .iter()
            .map(|sub| JobInfo {
                id: sub.id,
                date_submitted: systemtime_to_utc_string(&sub.date_submitted)
                    .unwrap_or("NO_TIME".to_string()),
                grading_tags: sub.grading_tags.split(";").map(String::from).collect(),
                //.map(|s| format!("<code>{s}</code>"))
                //.join(", "),
                status: SSC::from_i32(sub.exec_status_code)
                    .map_or("Unknown".to_string(), |c| format!("{c}")),
                status_symbol_and_class: SSC::from_i32(sub.exec_status_code).and_then(
                    |c| match c {
                        SSC::NotStarted | SSC::Running => None,
                        SSC::Success => Some((
                            settings.reporting.markdown.symbol_ok.to_owned(),
                            "text-success-emphasis".to_string(),
                        )),
                        _ => Some((
                            settings.reporting.markdown.symbol_failed.to_owned(),
                            "text-danger-emphasis".to_string(),
                        )),
                    },
                ),
            })
            .collect(),
    };
    let body: String = tpl
        .render_once()
        .map_err(|e| InternalError::new(e, StatusCode::INTERNAL_SERVER_ERROR))?;

    Ok(HttpResponse::Ok().body(body))
}

/// Template for showing job information
#[derive(TemplateSimple)]
#[template(path = "route/submission.stpl")]
struct SubmissionTemplate {
    // Common
    title: String,
    current_route: Option<String>,
    // For this template only
    submission_id: i64,
    report_markdown: String,
}

#[derive(Debug, Deserialize)]
struct SubmissionQuery {
    auth_key: String,
}

/// Display information about a single submission
async fn get_submission(
    settings: Settings,
    req: HttpRequest,
    submission_id: web::Path<String>,
) -> Result<HttpResponse, actix_web::Error> {
    use id2202_autograder::db::conn::DatabaseConnection;

    let q = match web::Query::<SubmissionQuery>::from_query(req.query_string()) {
        Ok(q) => q,
        Err(_) => {
            return ErrorMessageTemplate::unauthorized();
        }
    };

    let parsed_id: i64 = match submission_id.parse() {
        Ok(v) => v,
        Err(_) => {
            log::error!("Bad submission id: {submission_id}");
            return ErrorMessageTemplate::not_found();
        }
    };

    let mut conn = match DatabaseConnection::connect(&settings) {
        Ok(conn) => conn,
        Err(e) => {
            log::error!("Could not open database connection: {e}");
            return ErrorMessageTemplate::internal_server_error();
        }
    };

    let subinfo = match conn.get_submission_info(parsed_id) {
        Ok(subinfo) => subinfo,
        Err(_) => {
            log::error!("Submission id not found: {parsed_id}");
            return ErrorMessageTemplate::not_found();
        }
    };

    let sub = subinfo.get_submission();
    let src = subinfo.get_source();
    if src.auth_key != q.auth_key {
        return ErrorMessageTemplate::unauthorized();
    }

    let report_markdown = match &sub.exec_report {
        Some(v) => match Report::deserialize(v) {
            Ok(r) => r.to_markdown(&settings.reporting.markdown),
            Err(e) => format!("Invalid report: {:?}\n\n{}", e, v),
        },
        None => "No report generated.".to_string(),
    };

    let tpl = SubmissionTemplate {
        title: format!("Submission {}", sub.id),
        current_route: None,
        submission_id: sub.id,
        report_markdown: report_markdown,
    };
    let body: String = tpl
        .render_once()
        .map_err(|e| InternalError::new(e, StatusCode::INTERNAL_SERVER_ERROR))?;

    Ok(HttpResponse::Ok().body(body))
}
