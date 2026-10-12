//! Utilities for tearing down podman resources.

use crate::error::Error;
use std::ops::{Deref, DerefMut};

/// A trait indicating a resource that can be torn down.
pub trait Teardown {
    /// Tears down the underlying resource. The torn down resource should not
    /// be used again after invoking this function.
    fn teardown(&mut self) -> Result<(), Error>;
}

/// A handle that tears down the underlying podman resource on drop.
#[derive(Debug)]
pub struct OnDrop<T: Teardown>(T);

impl<T: Teardown> From<T> for OnDrop<T> {
    fn from(value: T) -> Self {
        OnDrop(value)
    }
}

impl<T: Teardown> Deref for OnDrop<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Teardown> DerefMut for OnDrop<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: Teardown> Drop for OnDrop<T> {
    fn drop(&mut self) {
        self.0
            .teardown()
            .unwrap_or_else(|e| log::warn!("could not tear down underlying resource: {e}"))
    }
}
