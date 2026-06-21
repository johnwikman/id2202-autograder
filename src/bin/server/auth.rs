/// This provides authentication middleware utilities for both route and api.
use actix_web::{
    body::MessageBody,
    dev::{ServiceRequest, ServiceResponse},
    middleware::Next,
    web::{self},
    HttpMessage,
};
use itertools::Itertools;
use serde::Deserialize;

use id2202_autograder::config::Settings;

// Names of recognized cookies
static COOKIE_NAME_API_AUTH_KEY: &str = "id2202_autograder_api_auth_key";

/// Struct with information about authorization for a request
#[derive(Clone)]
pub struct AuthorizationInfo {
    /// This indicates whether API authentication was successful. If this is
    /// true, then the used has provided an accepted API auth token.
    pub api_auth_ok: bool,

    /// The auth_key provided in the URL query
    /// (https://example.autograder.url/path?auth_key=KEY)
    ///
    /// This may be page specific, so this is validated by the requester.
    pub auth_key: Option<String>,
}

impl AuthorizationInfo {
    /// This returns true if and only if any authentication information is
    /// present that can be used to authenticate a request. If this is false,
    /// then there is no point in proceeding to authenticate.
    pub fn any_provided(&self) -> bool {
        self.api_auth_ok || self.auth_key.is_some()
    }
}

/// Middleware for authenticating a request.
pub async fn authenticate(
    data: web::Data<Settings>,
    req: ServiceRequest,
    next: Next<impl MessageBody>,
) -> Result<ServiceResponse<impl MessageBody>, actix_web::Error> {
    let settings = data.get_ref();

    let mut auth_info = AuthorizationInfo {
        api_auth_ok: false,
        auth_key: None,
    };

    // Check provided auth_key from the request query
    #[derive(Deserialize)]
    struct AuthQuery {
        auth_key: String,
    }
    match actix_web::web::Query::<AuthQuery>::from_query(req.query_string()) {
        Ok(q) => auth_info.auth_key = Some(q.auth_key.to_owned()),
        Err(_) => {}
    }

    // First check if this is provided as a header
    if let Some(auth_header) = req
        .headers()
        .get("Authorization")
        .and_then(|hv| hv.to_str().ok())
    {
        match auth_header.split_whitespace().collect_vec().as_slice() {
            &[method, key] => {
                if method.eq_ignore_ascii_case("bearer") {
                    if settings
                        .server
                        .secrets
                        .api_auth_tokens
                        .iter()
                        .any(|s| s == key)
                    {
                        auth_info.api_auth_ok = true;
                    }
                }
            }
            _ => {}
        };
    }

    // If not provided as a header, check if it is provided as a cookie
    if !auth_info.api_auth_ok {
        match req.cookie(COOKIE_NAME_API_AUTH_KEY) {
            Some(cookie) => {
                if settings
                    .server
                    .secrets
                    .api_auth_tokens
                    .iter()
                    .any(|s| s == cookie.value())
                {
                    auth_info.api_auth_ok = true;
                }
            }
            None => {}
        }
    }

    req.extensions_mut().insert(auth_info);
    let res = next.call(req).await?;
    Ok(res)
}
