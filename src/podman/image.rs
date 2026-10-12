//! Images in podman's default store.

use std::{collections::BTreeSet, time::Duration};

use serde::{Deserialize, Serialize};

use super::SHARED;
use crate::{error::Error, utils::SyscommandSettings};

/// A selection of JSON output fields when listing the images
#[derive(Serialize, Deserialize, Debug, Clone)]
struct ImageInfo {
    #[serde(rename = "Id")]
    pub id: String,

    #[serde(rename = "ParentId")]
    pub parent_id: String,

    #[serde(rename = "RepoDigests")]
    pub repo_digests: Vec<String>,

    #[serde(rename = "Size")]
    pub size: usize,

    #[serde(rename = "Digest")]
    pub digest: String,

    #[serde(rename = "History", default)]
    pub history: Vec<String>,

    #[serde(rename = "Names", default)]
    pub names: Vec<String>,
}

/// Returns a list of podman images on the system.
///
/// This has been tested using Podman 5.6.1
pub fn images() -> Result<Vec<String>, Error> {
    let imglist: Vec<ImageInfo> = SHARED.podman_cmd_json(["images", "--format", "json"])?;

    let imgs = BTreeSet::from_iter(imglist.into_iter().flat_map(|o| o.names.into_iter()));

    Ok(Vec::from_iter(imgs))
}

/// Pulls an image. This will time out after 20 minutes, which should be
/// sufficient for even the larger images.
pub fn pull(tag: &str) -> Result<(), Error> {
    SHARED.podman_cmd_with(
        ["pull", tag],
        SyscommandSettings {
            expected_code: Some(0),
            timeout: Duration::from_secs(1200),
            ..Default::default()
        },
    )?;
    Ok(())
}

/// Builds an image from `context_dir` and tags it `tag`. Shares the pull
/// timeout, since a build starts by pulling its base image. The output is
/// bounded rather than unlimited so that a failed build carries its log in the
/// returned error.
pub fn build(tag: &str, context_dir: &str, file: Option<&str>) -> Result<(), Error> {
    let mut args: Vec<&str> = vec!["build", "-t", tag];
    if let Some(f) = file {
        args.extend(["-f", f]);
    }
    args.push(context_dir);

    SHARED.podman_cmd_with(
        &args,
        SyscommandSettings {
            expected_code: Some(0),
            timeout: Duration::from_secs(1200),
            ..Default::default()
        },
    )?;

    Ok(())
}
