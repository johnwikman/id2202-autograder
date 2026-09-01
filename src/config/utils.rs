//! Common utilities for configuration handling.

use struct_patch::Patch;

/// A configuration value that an untreated deserialized override can be
/// applied to.
///
/// # Note
/// `P` should be the override type, which `#[derive(Patch)]` generates as a
/// copy of the value with every field optional. This is implemented for every such
/// pair, so a type only needs the derive.
pub trait ApplyUntreated<P>: Sized {
    /// Returns a copy of `self` with every field set by `untreated` replaced
    /// by that value. Anything not set by `untreated` remains unchanged.
    ///
    /// Providing `None` as the `untreated` value simply implies a clone of
    /// `self`.
    fn apply_untreated(&self, untreated: Option<P>) -> Self;
}

impl<T, P> ApplyUntreated<P> for T
where
    T: Patch<P> + Clone,
    P: Default,
{
    fn apply_untreated(&self, untreated: Option<P>) -> T {
        let mut applied = self.clone();
        applied.apply(untreated.unwrap_or_default());
        applied
    }
}
