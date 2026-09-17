//! The site landing page.

use maud::html;

use crate::components::Body;
use crate::route::ROUTES;

pub fn body(name: &str) -> Body {
    Body::new(html! {
        h1 { (name) " Documentation" }
        p {
            "Reference documentation for operating the autograder, generated directly from the \
             source so it stays in sync with the code. Use the navigation on the left, or the \
             links below:"
        }
        ul {
            @for route in ROUTES {
                @if let Some(description) = route.description {
                    li { a href=(route.file) { (route.title) } " - " (description) }
                }
            }
        }
    })
}
