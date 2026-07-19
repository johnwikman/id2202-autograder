//! One module per page of the generated site, each exposing a `render` that
//! returns the finished HTML. The pages are assembled from the crate's own
//! types (and, for the REST API, from the spec the server emits) so they cannot
//! drift from the code.

pub mod common;
pub mod index;
pub mod rest_api;
pub mod settings;
pub mod test_configuration;
