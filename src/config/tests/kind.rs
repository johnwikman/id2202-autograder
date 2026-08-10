//! Test kinds.
//!
//! Adding a kind means a module under `kind/`, an entry in the `kinds!`
//! invocation below, and a grader in the runner. Everything else — the
//! identifier list, the defaults lookup, the construction from a TOML table —
//! follows from that entry.

use serde::Deserialize;

use crate::{error::Error, utils::path_join};

pub mod check_file_exists;
pub mod gen_asm_and_run;
pub mod run;
pub mod run_verifier;

/// Context handed to a testkind once its options have been deserialized.
pub struct PostInitCtx<'a> {
    /// Absolute path to the directory containing the deserialized `.test.toml`
    /// file.
    pub dir: &'a str,

    /// Test case name, i.e. the filename without the `.test.toml` suffix.
    pub name: &'a str,
}

/// Work a kind performs after its options have been deserialized, such as
/// resolving paths relative to the file that declared them.
///
/// Deliberately has no blanket implementation: it is a supertrait of
/// [`TestKind`], so `#[derive(TestKind)]` does not compile until a kind has
/// considered what it needs here.
pub trait PostInit {
    fn post_init(&mut self, ctx: &PostInitCtx) -> Result<(), Error>;
}

/// Context for applying one configuration file's option overrides.
pub struct ApplyCtx<'a> {
    /// Absolute path to the directory holding the file that wrote the
    /// overrides, which `#[testkind(path)]` fields resolve against.
    pub dir: &'a str,
}

/// Describes each field within a kind, and how its option key may behave
/// beyond carrying a value, as declared by the `#[testkind(...)]` attributes
/// on its field.
pub struct FieldAttrs {
    pub name: &'static str,

    /// Key that masks this field, from `#[testkind(ignorable)]`.
    pub ignore_key: Option<&'static str>,

    /// `#[testkind(relpath)]`: resolved against the file the TOML file that
    /// specified it.
    pub is_relpath: bool,

    /// `#[testkind(clears(..))]`: fields reset whenever this one is set.
    pub clears: &'static [&'static str],

    /// `#[testkind(merge = "deep")]`: inherited tables merge key by key.
    pub deep_merge: bool,
}

/// Implemented by `#[derive(TestKind)]`.
pub trait TestKind: PostInit {
    /// Identifier used by `kind = "..."` and `[default.test.kinds.<ident>]`.
    const IDENT: &'static str;

    /// Information about every field within the test kind.
    const FIELDS: &'static [FieldAttrs];

    /// Applies the option overrides written by one configuration file. Rejects
    /// unknown keys, values of the wrong type, and `<key>_ignore = false`.
    fn apply(&mut self, overrides: &toml::Table, ctx: &ApplyCtx) -> Result<(), Error>;
}

macro_rules! kinds {
    ($($variant:ident => $module:ident::$ty:ident),+ $(,)?) => {
        /// A test case's kind together with its resolved options.
        #[derive(Debug, Clone)]
        pub enum Kind {
            $($variant($module::$ty),)+
        }

        impl Kind {
            /// Every registered identifier, in declaration order.
            pub fn idents() -> Vec<String> {
                vec![$(<$module::$ty as TestKind>::IDENT.to_string(),)+]
            }

            /// Selects one kind out of a resolved set of options.
            pub fn from_defaults(ident: &str, defaults: &KindDefaults) -> Result<Self, Error> {
                $(
                    if ident == <$module::$ty as TestKind>::IDENT {
                        return Ok(Self::$variant(defaults.$module.clone()));
                    }
                )+
                Error::err_identifier(ident, Self::idents())
            }
        }

        impl PostInit for Kind {
            fn post_init(&mut self, ctx: &PostInitCtx) -> Result<(), Error> {
                match self {
                    $(Self::$variant(t) => t.post_init(ctx),)+
                }
            }
        }

        /// The `[default.kind]` table: one complete set of option values per
        /// kind, which a test case's own options are merged into. The field
        /// name is the key used in the TOML.
        #[derive(Deserialize, Debug, Clone)]
        pub struct KindDefaults {
            $(pub $module: $module::$ty,)+
        }

        impl KindDefaults {
            /// Applies one file's overrides to the options of a single kind.
            pub fn apply(
                &mut self,
                ident: &str,
                overrides: &toml::Table,
                ctx: &ApplyCtx,
            ) -> Result<(), Error> {
                $(
                    if ident == <$module::$ty as TestKind>::IDENT {
                        return self.$module.apply(overrides, ctx);
                    }
                )+
                Error::err_identifier(ident, Kind::idents())
            }
        }
    };
}

kinds! {
    Run => run::Run,
    GenASMAndRun => gen_asm_and_run::GenASMAndRun,
    CheckFileExists => check_file_exists::CheckFileExists,
    RunVerifier => run_verifier::RunVerifier,
}

/// Resolves every entry of `dest` against the test directory, then appends the
/// files in that directory named `<name><suffix>` for each of `suffixes`.
pub fn discover_by_suffix(
    dest: &mut Vec<String>,
    suffixes: &[String],
    ctx: &PostInitCtx,
) -> Result<(), Error> {
    for f in dest.iter_mut() {
        *f = path_join(ctx.dir, &f)?;
    }
    let contents = std::fs::read_dir(ctx.dir).map_err(|e| {
        Error::fs("listing files for auto discovery", ctx.dir).with_cause(Box::new(e))
    })?;
    for entry in contents {
        let filename = entry?
            .file_name()
            .to_str()
            .map(String::from)
            .ok_or_else(|| Error::convert("Couldn't get string representation of DirEntry"))?;
        for suffix in suffixes {
            if let Some((p, "")) = filename.rsplit_once(suffix) {
                if p == ctx.name {
                    dest.push(path_join(ctx.dir, &filename)?);
                }
            }
        }
    }
    Ok(())
}
