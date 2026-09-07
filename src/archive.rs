use std::{
    collections::BTreeMap,
    fs::File,
    path::{Component, Path, PathBuf},
};

use read_restrict::ReadExt; // this provides `Read::restrict(u64)`

use crate::error::Error;

pub struct Archive {
    pub tree: BTreeMap<PathBuf, Vec<u8>>,
}

impl Archive {
    /// Create an archive from the `compressed` .tar.gz data. This allows it to
    /// expand into at most `limit` bytes of real data.
    pub fn from_targz(compressed: &[u8], limit: Option<u64>) -> Result<Self, Error> {
        use flate2::read::GzDecoder;
        use tar::Archive as TarArchive;

        let mut archive =
            TarArchive::new(GzDecoder::new(compressed).restrict(limit.unwrap_or(u64::MAX)));
        let mut tree = BTreeMap::new();

        for entry in archive.entries()? {
            let mut entry = entry?;
            if !entry.header().entry_type().is_file() {
                continue;
            }
            let path = entry.path()?.into_owned();
            if !path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)) {
                return Error::err_fs(
                    "the contained path will escape the archive root",
                    path.to_string_lossy(),
                );
            }
            let mut buf = Vec::new();
            std::io::copy(&mut entry, &mut buf).map_err(|e| {
                Error::fs(format!("could not read data from path: {e}"), path.to_string_lossy())
            })?;
            tree.insert(path, buf);
        }

        Ok(Self { tree })
    }

    /// Create an archive from the `compressed` .zip data. This allows it to
    /// expand into at most `limit` bytes of real data.
    pub fn from_zip(compressed: &[u8], limit: Option<u64>) -> Result<Self, Error> {
        use zip::ZipArchive;

        let mut zip = ZipArchive::new(std::io::Cursor::new(compressed)).map_err(|e| {
            Error::convert("could not initialize ZipArchive").with_cause(e)
        })?;
        let mut tree = BTreeMap::new();
        let mut budget = limit.unwrap_or(u64::MAX);

        for i in 0..zip.len() {
            let entry = zip.by_index(i).map_err(|e| {
                Error::runtime("FATAL: internal error in Archive::from_zip").with_cause(e)
            })?;
            if entry.is_dir() {
                continue;
            }
            let path = entry.enclosed_name().ok_or_else(|| {
                Error::fs("the contained path will escape the archive root", entry.name())
            })?;

            let mut src = entry.restrict(budget);

            let mut buf = Vec::new();
            std::io::copy(&mut src, &mut buf).map_err(|e| {
                Error::fs(format!("could not read data from path: {e}"), path.to_string_lossy())
            })?;
            tree.insert(path, buf);
            budget = src.restriction();
        }

        Ok(Self { tree })
    }

    /// Writes this archive into a .tar.gz file at the specified destination.
    pub fn to_targz(&self, dest: impl AsRef<Path>) -> Result<(), Error> {
        use flate2::{write::GzEncoder, Compression};
        use tar::{Builder as TarBuilder, Header};

        let dest = dest.as_ref();
        if !dest.to_str().is_some_and(|s| s.ends_with(".tar.gz")) {
            log::warn!(
                "writing to a file that does not end with .tar.gz: \"{}\"",
                dest.to_string_lossy()
            );
        }

        let f = File::create(dest).map_err(|e| {
            Error::fs("could not create .tar.gz destination file", dest.to_string_lossy())
                .with_cause(e)
        })?;
        let mut tb = TarBuilder::new(GzEncoder::new(f, Compression::default()));

        for (path, data) in &self.tree {
            let mut header = Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_mtime(0);
            tb.append_data(&mut header, path, data.as_slice())?;
        }

        tb.into_inner()?.finish()?.sync_all()?;

        Ok(())
    }
}
