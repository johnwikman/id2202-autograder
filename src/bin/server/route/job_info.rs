use actix_web::{
    error::InternalError,
    http::StatusCode,
    web::{self},
    HttpMessage, HttpRequest, HttpResponse,
};
use sailfish::TemplateSimple;

use id2202_autograder::{config::Settings, utils::utc_string};

use crate::{
    auth::AuthorizationInfo,
    route::{common::CommonInformation, error_msg},
};

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
    grading_tags: &'a [String],
    status: String,
    /// ("symbol", "span class")
    status_symbol_and_class: (Option<String>, Option<String>),
    /// Link to the submission page (link is hidden if None)
    href: Option<String>,
}

/// Page that shows information about current jobs.
pub async fn get_job_info(
    current_route: &str,
    data: web::Data<Settings>,
    req: HttpRequest,
) -> Result<HttpResponse, actix_web::Error> {
    use id2202_autograder::db::{
        conn::DatabaseConnection, models::raw::SubmissionRow, models::Submission,
        models::SubmissionStatus,
    };

    let settings = data.get_ref();

    // This has no security implication, only that we show links if we know for
    // certain that the user is authenticated.
    let show_submission_links =
        req.extensions().get::<AuthorizationInfo>().is_some_and(|a| a.api_auth_ok);

    let mut conn = match DatabaseConnection::connect(settings) {
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
        let rows: Vec<SubmissionRow> = match submissions::table
            .select(SubmissionRow::as_select())
            .order(id.desc())
            .limit(100)
            .load(&mut conn.conn)
        {
            Ok(v) => v,
            Err(e) => {
                log::error!("Could not get submissions from database: {e}");
                return error_msg::internal_server_error(settings);
            }
        };
        match Submission::from_rows(&mut conn, rows) {
            Ok(v) => v,
            Err(e) => {
                log::error!("Could not assemble the submissions: {e}");
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
                date_submitted: utc_string(&sub.submitted_at),
                grading_tags: &sub.requested_tags,
                status: sub.status().to_string(),
                status_symbol_and_class: match sub.status() {
                    SubmissionStatus::Waiting | SubmissionStatus::InProgress => {
                        (Some(settings.reporting.markdown.symbol_waiting.clone()), None)
                    }
                    SubmissionStatus::Success => (
                        Some(settings.reporting.markdown.symbol_ok.clone()),
                        Some("text-success-emphasis".to_string()),
                    ),
                    SubmissionStatus::Failed
                    | SubmissionStatus::Aborted
                    | SubmissionStatus::Unknown => (
                        Some(settings.reporting.markdown.symbol_failed.clone()),
                        Some("text-danger-emphasis".to_string()),
                    ),
                },
                href: show_submission_links.then(|| format!("/submission/{}", sub.id)),
            })
            .collect(),
    };
    let body: String =
        tpl.render_once().map_err(|e| InternalError::new(e, StatusCode::INTERNAL_SERVER_ERROR))?;

    Ok(HttpResponse::Ok().body(body))
}
