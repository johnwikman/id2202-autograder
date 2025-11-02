use actix_web::{
    error::InternalError,
    get,
    http::StatusCode,
    web::{self, ServiceConfig},
    HttpRequest, HttpResponse, HttpResponseBuilder, Responder,
};
use num_traits::FromPrimitive;
use sailfish::TemplateSimple;

use id2202_autograder::{settings::Settings, utils::systemtime_to_utc_string};

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
                            settings.runner.md_settings.symbol_ok.to_owned(),
                            "text-success-emphasis".to_string(),
                        )),
                        _ => Some((
                            settings.runner.md_settings.symbol_failed.to_owned(),
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
