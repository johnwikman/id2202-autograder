use actix_web::{
    web::{self},
    HttpMessage, HttpRequest, Responder,
};

use id2202_autograder::{
    config::Settings,
    db::{conn::DatabaseConnection, models::SubmissionWithReport},
};

use crate::{
    api::response::{ErrorResponse, SubmissionResponse},
    auth::AuthorizationInfo,
};

/// Fetching submissions from the database
///
/// Required headers:
///  - Authorization
pub async fn get_submission(
    data: web::Data<Settings>,
    req: HttpRequest,
    submission_id: web::Path<String>,
) -> Result<impl Responder, actix_web::Error> {
    use diesel::{self, ExpressionMethods, QueryDsl, RunQueryDsl, SelectableHelper};
    use id2202_autograder::db::schema::submissions::{self, columns as sub_col};

    let settings = data.get_ref();

    let auth_info = req
        .extensions()
        .get::<AuthorizationInfo>()
        .ok_or_else(|| ErrorResponse::unauthorized(&req, "missing Authorization header"))?
        .clone();
    if !auth_info.api_auth_ok {
        // API authentication failed
        return Err(ErrorResponse::unauthorized(&req, "API authentication failed").into());
    }

    // Request is Authorized
    let parsed_id: i64 = match submission_id.parse() {
        Ok(v) => v,
        Err(_) => {
            log::error!("Bad submission id: {submission_id}");
            return Err(ErrorResponse::bad_request(&req, "bad submission id format").into());
        }
    };

    let mut conn = match DatabaseConnection::connect(&settings) {
        Ok(conn) => conn,
        Err(e) => {
            log::error!("Could not open database connection: {e}");
            return Err(ErrorResponse::internal_server_error(&req).into());
        }
    };

    let swr: SubmissionWithReport = submissions::table
        .select(SubmissionWithReport::as_select())
        .filter(sub_col::id.eq(parsed_id))
        .first(&mut conn.conn)
        .map_err(|e: diesel::result::Error| {
            log::error!(
                "could not get submission {parsed_id} with report from database: {:?}",
                e
            );
            ErrorResponse::internal_server_error(&req)
        })?;

    Ok(SubmissionResponse::new(&req, &swr).to_http())
}
