//! Tags: the unit of grading. A tag says what to build and which directories
//! hold its test cases.

use documented::{Documented, DocumentedFields};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_value::Value as SerdeValue;
use std::collections::BTreeMap;
use struct_patch::Patch;

use super::{group::TestGroup, Defaults, TestsLoadingOptions};
use crate::{config::utils::ApplyUntreated, error::Error, utils::path_absolute_join};

/// Defaults applying to a tag as a whole.
#[derive(Deserialize, JsonSchema, Debug, Clone, Documented, DocumentedFields)]
pub struct TagDefaults {
    /// Default timeout (in seconds) of a tag as a whole.
    pub timeout_total: u32,

    /// Default rate limits for tags.
    pub rate_limit: RateLimit,

    /// Default budged of tags.
    pub budget: Budget,
}

/// Limits how often a tag can be graded from a single source.
#[derive(
    Patch,
    Deserialize,
    Serialize,
    JsonSchema,
    Debug,
    Clone,
    Documented,
    DocumentedFields,
    utoipa::ToSchema,
)]
#[patch(name = "_UntreatedRateLimit", attribute(derive(Deserialize, Default)))]
pub struct RateLimit {
    /// Whether the limit applies. If false, this tag will not be rate-limited.
    pub enable: bool,

    /// How many runs of the tag can be graded within the space of a window.
    pub n: u32,

    /// Length of the window, in seconds.
    pub window_seconds: u64,
}

/// Limits how many times in total a tag can be graded from a single source.
#[derive(
    Patch,
    Deserialize,
    Serialize,
    JsonSchema,
    Debug,
    Clone,
    Documented,
    DocumentedFields,
    utoipa::ToSchema,
)]
#[patch(name = "_UntreatedBudget", attribute(derive(Deserialize, Default)))]
pub struct Budget {
    /// Whether the budget applies. If false, this tag has no limit on the
    /// number of times it can be graded.
    pub enable: bool,

    /// How many runs of the tag can be graded before the rest are rejected.
    pub max_runs: u32,
}

/// Configuration for how to build the project that is being graded by a tag.
#[derive(
    Patch,
    Deserialize,
    Serialize,
    JsonSchema,
    Debug,
    Clone,
    Documented,
    DocumentedFields,
    utoipa::ToSchema,
)]
#[patch(name = "_UntreatedBuildConfig", attribute(derive(Deserialize, Default)))]
pub struct BuildConfig {
    /// The source directory that contains the files to build.
    pub srcdir: String,

    /// The command used for building the project once located in the project
    /// folder.
    pub cmd: Vec<String>,

    /// Timeout (in seconds) for building the project.
    pub timeout: u32,

    /// Maximum captured output on stdout and stderr for the build, in bytes.
    pub max_output: usize,

    /// If this is true, then the runner will give a build error if there are
    /// any binary files present in the build directory.
    pub prohibit_binary_files: bool,

    /// If prohibit_binary_files is true, then this specifies a list of
    /// exceptions. I.e. binary files that should still be allowed.
    pub allowed_binary_files: Vec<String>,

    /// Additional MIME types that do not begin with `"text/"` that shall be
    /// allowed regardless.
    pub allowed_binary_mimetypes: Vec<String>,
}

/// A test tag that can be invoked and graded.
#[derive(Debug, Clone)]
pub struct Tag {
    /// Tag name identifier.
    pub name: String,

    /// Optional path to a file containing the instructions for the task that
    /// this is running the test cases for. This is an optional field, and the
    /// content of this file is opaque to the autograder. If provided, the
    /// autograder will check that this actually points to a file.
    pub task_file: Option<String>,

    /// Opaque metadata for the tag. This will never be inspected by the
    /// autograder.
    pub metadata: BTreeMap<String, SerdeValue>,

    /// The tests contained within this tag.
    pub test_groups: Vec<TestGroup>,

    /// Config on how the build the project being graded.
    pub build: BuildConfig,

    /// How often this tag can be graded from a single source.
    pub rate_limit: RateLimit,

    /// How many times in total this tag can be graded from a single source.
    pub budget: Budget,
}

/// A deserializable version of `Tag`, which does not extend any other tag.
#[derive(Deserialize)]
struct _UntreatedTag {
    dirs: Vec<String>,
    build: _UntreatedBuildConfig,
    metadata: Option<BTreeMap<String, SerdeValue>>,
    task_file: Option<String>,
    rate_limit: Option<_UntreatedRateLimit>,
    budget: Option<_UntreatedBudget>,
}

/// An `Tag` which extends a previous tag, inheriting values from another tag
/// specified by the `extends` field. Note that the `dirs` field adds to the
/// previously specified directory, and `metadata` is shallowly merged with the
/// previous one.
#[derive(Deserialize)]
struct _UntreatedExtensibleTag {
    extends: String,
    dirs: Vec<String>,
    metadata: Option<BTreeMap<String, SerdeValue>>,
    task_file: Option<String>,
    rate_limit: Option<_UntreatedRateLimit>,
    budget: Option<_UntreatedBudget>,
}

impl Tag {
    /// Instantiates `tag_definitions` in TOML form to a complete `Tag`,
    /// handling behavior such as extensible tags and values being inherited
    /// from `defaults`.
    ///
    /// The reason for provided all defined tags at once through the
    /// `tag_definitions` parameter is to resolve extension dependencies. Since
    /// the order does not matter when defining the tags, all tags must be
    /// provided together and resolved as a unit.
    ///
    /// The `config_path` parameter is just there for logging purposes, stating
    /// the path where the TOML definitions originate from.
    pub fn from_toml(
        mut tag_definitions: BTreeMap<String, toml::Value>,
        defaults: &Defaults,
        root_dir: &str,
        config_path: &str,
        options: &TestsLoadingOptions,
    ) -> Result<BTreeMap<String, Tag>, Error> {
        let task_file = |tag: &str, p: Option<String>| -> Result<Option<String>, Error> {
            let Some(p) = p else {
                return Ok(None);
            };
            let abs_path = path_absolute_join(root_dir, &p)?;
            if !std::fs::exists(&abs_path)? {
                return Err(Error::test_config_msg(format!("task file \"{abs_path}\" not found"))
                    .tag(tag)
                    .path(config_path)
                    .into());
            }
            Ok(Some(abs_path))
        };

        log::debug!("Extracting each tag configuration");
        // Tags without their test groups, paired with the directories that hold
        // the test cases to scan into those groups.
        let mut unscanned: BTreeMap<String, (Tag, Vec<String>)> = BTreeMap::new();
        while !tag_definitions.is_empty() {
            let mut found: Vec<String> = vec![];

            for (name, data) in tag_definitions.iter() {
                if !tag_is_valid(name) {
                    return Err(Error::test_config_msg("invalid tag name")
                        .tag(name)
                        .path(config_path)
                        .into());
                }
                match data {
                    toml::Value::Table(t) => {
                        if t.contains_key("extends") {
                            let uetg: _UntreatedExtensibleTag =
                                data.to_owned().try_into().map_err(Error::from)?;
                            if let Some((extended, extended_dirs)) = unscanned.get(&uetg.extends) {
                                let mut metadata = extended.metadata.to_owned();
                                if let Some(m) = uetg.metadata {
                                    metadata.extend(m);
                                }
                                let t = Tag {
                                    name: name.to_owned(),
                                    // The extended tag carries an already
                                    // resolved and checked path.
                                    task_file: task_file(name, uetg.task_file)?
                                        .or_else(|| extended.task_file.to_owned()),
                                    metadata,
                                    test_groups: vec![],
                                    build: extended.build.to_owned(),
                                    rate_limit: extended
                                        .rate_limit
                                        .apply_untreated(uetg.rate_limit),
                                    budget: extended.budget.apply_untreated(uetg.budget),
                                };
                                let dirs = [extended_dirs.to_owned(), uetg.dirs].concat();
                                log::debug!("Found tag {t:?}");
                                found.push(name.to_owned());
                                unscanned.insert(name.to_owned(), (t, dirs));
                            }
                        } else {
                            // This is a root tag that doesn't extend anything
                            let utg: _UntreatedTag =
                                data.to_owned().try_into().map_err(Error::from)?;
                            let t = Tag {
                                name: name.to_owned(),
                                task_file: task_file(name, utg.task_file)?,
                                metadata: utg.metadata.unwrap_or_default(),
                                test_groups: vec![],
                                build: defaults.build.apply_untreated(Some(utg.build)),
                                rate_limit: defaults.tag.rate_limit.apply_untreated(utg.rate_limit),
                                budget: defaults.tag.budget.apply_untreated(utg.budget),
                            };
                            log::debug!("Found tag {t:?}");
                            found.push(name.to_owned());
                            unscanned.insert(name.to_owned(), (t, utg.dirs));
                        }
                    }
                    _ => {
                        return Err(Error::test_config_msg("tag specification must be a table")
                            .tag(name)
                            .path(config_path)
                            .into());
                    }
                }
            }

            if found.is_empty() {
                return Err(Error::test_config_msg(format!(
                    "Could not instantiate tag configuration. Remaining keys: {tag_definitions:?}"
                ))
                .path(config_path)
                .into());
            } else {
                log::debug!("Removing all found keys");
                for k in found {
                    tag_definitions.remove(&k);
                }
            }
        }

        if options.taginfo_only {
            log::debug!("Skipping the loading of test groups due to the taginfo_only flag.");
            return Ok(unscanned.into_iter().map(|(k, (t, _))| (k, t)).collect());
        }

        log::debug!("Scanning the directories for each grading tag.");
        unscanned
            .into_iter()
            .map(|(name, (mut t, dirs))| {
                log::debug!("Converting each directory of tag \"{name}\" to a test group");
                for dir in dirs {
                    log::debug!("Scanning directory {dir}");
                    let absdir = path_absolute_join(root_dir, &dir)?;
                    t.test_groups.push(TestGroup::new(&absdir, &defaults.test, vec![])?);
                }
                Ok((name, t))
            })
            .collect()
    }
}

/// Matches the string `s` if it contains any leading tag. Returns a tuple with
/// the matched tag and the remaining text after the tag.
pub fn tag_match(s: &str) -> (&str, &str) {
    for (i, c) in s.char_indices() {
        // Could technically use a regex here instead, but could not find a
        // lightweight and well-documented regex library that allows for
        // compile-time evaluation.
        if !matches!(c, '0'..='9' | 'a'..='z' | 'A'..='Z' | '-' | '_') {
            // Invalid character, here the tag ends.
            return s.split_at(i);
        }
    }
    (s, "")
}

/// Returns `true` if the tag is valid, otherwise `false`.
pub fn tag_is_valid(tag: &str) -> bool {
    if tag.is_empty() {
        return false;
    }
    let (_, rest) = tag_match(tag);
    rest.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test cases for tag_match and tag_is_valid.
    /// Each tuple is (input, expected_tag, expected_remainder).
    /// tag_is_valid is derived: true when remainder is empty.
    const TAG_TEST_CASES: &[(&str, &str, &str)] = &[
        // Basic cases
        ("hello 5", "hello", " 5"),
        (" hello 5", "", " hello 5"),
        ("some-thing. else", "some-thing", ". else"),
        ("A_B_c_Zz-009", "A_B_c_Zz-009", ""),
        ("hello", "hello", ""),
        ("hello5", "hello5", ""),
        // Single characters
        ("a", "a", ""),
        ("Z", "Z", ""),
        ("5", "5", ""),
        ("-", "-", ""),
        ("_", "_", ""),
        (".", "", "."),
        // Numbers at start
        ("123abc", "123abc", ""),
        ("123", "123", ""),
        ("0-tag", "0-tag", ""),
        ("1st-tag", "1st-tag", ""),
        // Only special valid characters
        ("---", "---", ""),
        ("___", "___", ""),
        ("-_-_-", "-_-_-", ""),
        ("-_-", "-_-", ""),
        // Whitespace delimiters
        ("tag\ttab", "tag", "\ttab"),
        ("tag\nnewline", "tag", "\nnewline"),
        ("tag\r\nwindows", "tag", "\r\nwindows"),
        ("tag\n", "tag", "\n"),
        ("\t", "", "\t"),
        ("\n", "", "\n"),
        ("\r\n", "", "\r\n"),
        (" ", "", " "),
        ("  ", "", "  "),
        ("\t\t", "", "\t\t"),
        (" tag", "", " tag"),
        ("\ttag", "", "\ttag"),
        ("\ntag", "", "\ntag"),
        ("tag ", "tag", " "),
        ("tag\t", "tag", "\t"),
        ("tag \t\n", "tag", " \t\n"),
        ("a b c", "a", " b c"),
        ("tag\x0b", "tag", "\x0b"), // vertical tab
        ("tag\x0c", "tag", "\x0c"), // form feed
        // Unicode characters (should stop at them)
        ("tagåäö", "tag", "åäö"),
        ("héllo", "h", "éllo"),
        ("日本語", "", "日本語"),
        ("tägname", "t", "ägname"),
        ("tag™", "tag", "™"),
        // Special characters as delimiters
        ("tag@email", "tag", "@email"),
        ("tag/path", "tag", "/path"),
        ("tag:value", "tag", ":value"),
        ("tag=value", "tag", "=value"),
        ("tag.name", "tag", ".name"),
        ("!! hello", "", "!! hello"),
        ("#hello", "", "#hello"),
    ];

    #[test]
    fn test_tag_match() {
        // Empty string is a special case
        assert_eq!(tag_match(""), ("", ""));

        for &(input, expected_tag, expected_rest) in TAG_TEST_CASES {
            assert_eq!(tag_match(input), (expected_tag, expected_rest), "tag_match({input:?})");
        }
    }

    #[test]
    fn test_tag_is_valid() {
        // Empty string is a special case: not a valid tag
        assert!(!tag_is_valid(""));

        for &(input, _, expected_rest) in TAG_TEST_CASES {
            let expected_valid = expected_rest.is_empty();
            assert_eq!(tag_is_valid(input), expected_valid, "tag_is_valid({input:?})");
        }
    }
}
