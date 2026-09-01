//! The shadow repository, which archives what was graded and what came of it.

use std::time::Duration;

use id2202_autograder::{
    config::Settings,
    db::models::{origin::StoredOriginEnum, Submission},
    error::Error,
    reporting::ReportTagGrading,
    utils::{
        create_dir_if_not_exists, fsfriendly_utc_string, path_absolute_join, path_absolute_parent,
        syscommand_timeout, write_all_timeout, SyscommandSettings,
    },
};

/// A clone of one submission source's shadow repository. Every write goes to
/// the clone, and reaches the bare repository when [ShadowRepo::commit] pushes
/// it.
#[derive(Debug)]
pub struct ShadowRepo<'a> {
    settings: &'a Settings,

    /// The bare repository that everything is pushed to.
    bare: String,

    /// The clone that files are written into.
    clone: String,

    /// Where this submission's results are collected, named after the time of
    /// submission.
    date_dir: String,

    /// Where the graded source is kept. Unlike `date_dir` this holds only the
    /// most recent grading, so it is overwritten rather than added to.
    snapshot_dir: String,

    /// The submission this was opened for.
    pub submission_id: i64,
}

impl<'a> ShadowRepo<'a> {
    /// Creates the bare repository if it does not exist yet, and clones it into
    /// `workspace_dir`.
    pub fn open<J>(
        settings: &'a Settings,
        sub: &Submission<J>,
        workspace_dir: &str,
    ) -> Result<Self, Error> {
        let bare = match &sub.origin.origin {
            StoredOriginEnum::GitHub(gh) => path_absolute_join(
                &settings.runner.shadow_dir,
                format!("github/{}/{}/{}.git", gh.src.domain, gh.src.org, gh.src.repo),
            )?,
            StoredOriginEnum::GitLab(gl) => path_absolute_join(
                &settings.runner.shadow_dir,
                format!("gitlab/{}/{}/{}.git", gl.src.domain, gl.src.namespace, gl.src.repo),
            )?,
        };

        if !std::fs::exists(&bare)? {
            log::info!(
                "The shadow repository does not exist. Creating new shadow repository at path {bare}"
            );
            std::fs::create_dir_all(&bare)?;

            syscommand_timeout(
                ["git", "-C", &bare, "init", "--bare"],
                SyscommandSettings { expected_code: Some(0), ..Default::default() },
            )?;
        }

        create_dir_if_not_exists(workspace_dir)?;
        let clone = path_absolute_join(workspace_dir, "shadow")?;

        // Note: We must ensure that we don't use hardlinks when cloning as the
        // shadow repo might be on a mounted filesystem in docker.
        log::debug!("Cloning shadow directory {bare} to {clone}");
        syscommand_timeout(
            ["git", "clone", "--no-hardlinks", &bare, &clone],
            SyscommandSettings { expected_code: Some(0), ..Default::default() },
        )
        .inspect_err(|e| log::error!("Could not clone shadow repo {bare}: {e}"))?;

        for (key, value) in
            [("user.name", settings.name.as_str()), ("user.email", "id2202@localhost")]
        {
            syscommand_timeout(
                ["git", "-C", &clone, "config", "--local", key, value],
                SyscommandSettings { expected_code: Some(0), ..Default::default() },
            )
            .inspect_err(|e| log::error!("Could not set git config for shadow repo: {e}"))?;
        }

        // A submission graded over more than one claim shares this directory,
        // so the later ones must not fail on finding it already there.
        let date_dir = path_absolute_join(&clone, fsfriendly_utc_string(&sub.submitted_at))?;
        std::fs::create_dir_all(&date_dir)?;

        let snapshot_dir = path_absolute_join(&clone, "snapshot")?;

        Ok(ShadowRepo { settings, bare, clone, date_dir, snapshot_dir, submission_id: sub.id })
    }

    /// Writes the grading result of the tag the report was produced for.
    pub fn write_result(&self, report: &ReportTagGrading) -> Result<(), Error> {
        let path = path_absolute_join(&self.date_dir, format!("{}.results.json", report.tag_name))?;
        let mut f = std::fs::File::create(path)?;
        write_all_timeout(
            &mut f,
            report.to_json()?.as_bytes(),
            Duration::from_secs(self.settings.fs_write_timeout_seconds.into()),
        )?;
        Ok(())
    }

    /// Replaces the snapshot of `tag_srcdir` with the copy of it in
    /// `origin_repo`, where the submitted source was checked out.
    /// `tag_srcdir` is relative to that checkout.
    pub fn snapshot(&self, origin_repo: &str, tag_srcdir: &str) -> Result<(), Error> {
        let from = path_absolute_join(origin_repo, tag_srcdir)?;
        let to = path_absolute_join(&self.snapshot_dir, tag_srcdir)?;
        let parent = path_absolute_parent(&to)?;

        if !std::fs::exists(&parent)? {
            std::fs::create_dir_all(&parent)?;
        }
        if std::fs::exists(&to)? {
            std::fs::remove_dir_all(&to)?;
        }

        dircpy::copy_dir(&from, &to)?;
        Ok(())
    }

    /// Commits everything written since the last commit, and pushes it to the
    /// bare repository.
    pub fn commit(&self, commit_msg: &str) -> Result<(), Error> {
        let mut cmdadd: Vec<&str> = vec!["git", "-C", &self.clone, "add", &self.date_dir];
        if std::fs::exists(&self.snapshot_dir)? {
            cmdadd.push(&self.snapshot_dir);
        }

        syscommand_timeout(
            cmdadd.as_slice(),
            SyscommandSettings { expected_code: Some(0), ..Default::default() },
        )
        .inspect_err(|e| log::error!("Could not add files to shadow repo {}: {e}", self.bare))?;

        syscommand_timeout(
            ["git", "-C", &self.clone, "commit", "--allow-empty", "-m", commit_msg],
            SyscommandSettings { expected_code: Some(0), ..Default::default() },
        )
        .inspect_err(|e| log::error!("Could not commit files to shadow repo {}: {e}", self.bare))?;

        self.push()
    }

    /// Pushes the clone, rebasing once if the bare repository has moved on.
    fn push(&self) -> Result<(), Error> {
        let push = || {
            syscommand_timeout(
                ["git", "-C", &self.clone, "push"],
                SyscommandSettings { expected_code: Some(0), ..Default::default() },
            )
        };

        let Err(e) = push() else {
            return Ok(());
        };

        // One runner holds a source for as long as any of its jobs is
        // outstanding, so nothing else should be able to push here. Reaching
        // this point means that exclusion has already been broken, and the
        // rebase only keeps the student's grading alive.
        log::error!(
            "Could not push to shadow repo {}: {e}. Another writer has touched it, which should \
             be impossible. Rebasing and retrying.",
            self.bare
        );

        syscommand_timeout(
            ["git", "-C", &self.clone, "pull", "--rebase"],
            SyscommandSettings { expected_code: Some(0), ..Default::default() },
        )
        .inspect_err(|e| log::error!("Could not rebase shadow repo {}: {e}", self.bare))?;

        push().inspect_err(|e| {
            log::error!("Could not push to shadow repo {} after rebasing: {e}", self.bare)
        })?;

        Ok(())
    }
}
