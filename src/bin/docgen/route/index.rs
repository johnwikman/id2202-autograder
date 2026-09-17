//! The site landing page.

use maud::html;

use crate::components::Body;
use crate::route::ROUTES;

pub fn body(name: &str) -> Body {
    Body::new(html! {
        h1 { (name) " Documentation" }
        p {
            "Reference documentation for using the autograder, generated directly from the \
             source code. Use the navigation on the left, or the links below:"
        }
        ul {
            @for route in ROUTES {
                @if let Some(description) = route.description {
                    li { a href=(route.file) { (route.title) } " - " (description) }
                }
            }
        }
        p {
            "For development instructions, please see the README.md file in \
             the root of the repository."
        }
    })
}
