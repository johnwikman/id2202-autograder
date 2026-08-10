//! Derive macros for the autograder.
//!
//! Each module mirrors the path of the code it serves, so the derive for
//! `config::tests::kind` lives in `config/tests/kind.rs`. Derive entry points
//! may only appear at the crate root, so this file is a thin list of them.

mod config;

use proc_macro::TokenStream;

/// Derives `config::tests::kind::TestKind` for a test kind options struct.
///
/// ```ignore
/// #[derive(TestKind)]
/// #[testkind(ident = "run")]
/// pub struct Run { /* ... */ }
/// ```
#[proc_macro_derive(TestKind, attributes(testkind))]
pub fn derive_test_kind(input: TokenStream) -> TokenStream {
    config::tests::kind::derive(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
