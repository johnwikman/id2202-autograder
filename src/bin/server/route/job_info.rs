use actix_web::{
    error::InternalError,
    http::StatusCode,
    web::{self},
    HttpRequest, HttpResponse,
};
use num_traits::FromPrimitive;
use sailfish::TemplateSimple;

use id2202_autograder::{config::Settings, utils::systemtime_to_utc_string};

use crate::route::{common::CommonInformation, error_msg};

/// Template for showing job information
#[derive(TemplateSimple)]
#[template(path = "route/job_info.stpl")]
struct JobInfoTemplate<'a> {
    common: CommonInformation,
    // For this template only
    job_info_vec: Vec<JobInfo<'a>>,
}

struct JobInfo<'a> {
    id: i64,
    date_submitted: String,
    grading_tags: Vec<&'a str>,
    status: String,
    /// ("symbol", "span class")
    status_symbol_and_class: Option<(String, String)>,
}

/// Page that shows information about current jobs.
pub async fn get_job_info(
    current_route: &str,
    data: web::Data<Settings>,
    _req: HttpRequest,
) -> Result<HttpResponse, actix_web::Error> {
    use id2202_autograder::db::{
        conn::DatabaseConnection, models::Submission, models::SubmissionStatusCode as SSC,
    };

    let settings = data.get_ref();

    let mut conn = match DatabaseConnection::connect(&settings) {
        Ok(conn) => conn,
        Err(e) => {
            log::error!("Could not open database connection: {e}");
            return error_msg::internal_server_error(settings);
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
                return error_msg::internal_server_error(settings);
            }
        }
    };

    let tpl = JobInfoTemplate {
        common: CommonInformation::from_title_route(settings, "Job Info", current_route),
        job_info_vec: subs
            .iter()
            .map(|sub| JobInfo {
                id: sub.id,
                date_submitted: systemtime_to_utc_string(&sub.date_submitted)
                    .unwrap_or("NO_TIME".to_string()),
                grading_tags: sub.grading_tags.split(";").collect(),
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
