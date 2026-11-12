//! Validate policy ownership and retain the exact opened source for revalidation.
//! Same-UID malicious writers remain outside the boundary; detectable replacement
//! or modification between loading and launch is nevertheless a hard error.

#[cfg(unix)]
mod platform {
    use anyhow::{Context, Result, bail};
    #[cfg(target_os = "macos")]
    use std::os::unix::ffi::OsStringExt;
    use std::os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    };
    use std::{
        fs::{self, File, Metadata, OpenOptions},
        io::Read,
        path::{Path, PathBuf},
    };

    /// Identity and modification stamp obtained from fstat, never from TOML contents.
    #[derive(Debug, PartialEq, Eq)]
    struct Stamp {
        device: u64,
        inode: u64,
        owner: u32,
        mode: u32,
        length: u64,
        modified: (i64, i64),
        changed: (i64, i64),
    }

    impl Stamp {
        /// Include timestamps so in-place writes are detected as well as replacements.
        fn from(metadata: &Metadata) -> Self {
            Self {
                device: metadata.dev(),
                inode: metadata.ino(),
                owner: metadata.uid(),
                mode: metadata.mode(),
                length: metadata.len(),
                modified: (metadata.mtime(), metadata.mtime_nsec()),
                changed: (metadata.ctime(), metadata.ctime_nsec()),
            }
        }
    }

    /// An opened source and both its selected and descriptor-derived path spellings.
    #[derive(Debug)]
    pub struct Source {
        pub path: PathBuf,
        pub target: PathBuf,
        file: File,
        stamp: Stamp,
        allow_shared: bool,
    }

    /// Require trusted owners and no group/other writes. Root-owned sticky ancestors
    /// such as /tmp are allowed, but a policy directory itself must be private to its owner.
    fn permissions(path: &Path, metadata: &Metadata, sticky_ancestor: bool) -> Result<()> {
        let owner = metadata.uid();
        let mode = metadata.mode();
        let sticky = sticky_ancestor && metadata.is_dir() && owner == 0 && mode & 0o1000 != 0;
        if (owner != unsafe { libc::geteuid() } && owner != 0) || (mode & 0o022 != 0 && !sticky) {
            bail!(
                "untrusted profile path {}: require effective-user/root ownership and no group/other writes (use --allow-shared-profiles only for intentionally shared policy)",
                path.display()
            );
        }
        Ok(())
    }

    /// Check lexical and resolved ancestors, including symlink parents and absent roots.
    pub fn directory(path: &Path, allow_shared: bool) -> Result<()> {
        if allow_shared {
            return Ok(());
        }
        for ancestor in path.ancestors() {
            match fs::metadata(ancestor) {
                Ok(metadata) => {
                    if !metadata.is_dir() {
                        bail!("profile parent is not a directory: {}", ancestor.display());
                    }
                    permissions(ancestor, &metadata, ancestor != path)?;
                    let canonical = fs::canonicalize(ancestor)?;
                    for resolved in canonical.ancestors() {
                        permissions(
                            resolved,
                            &fs::metadata(resolved)?,
                            resolved != canonical || ancestor != path,
                        )?;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if fs::symlink_metadata(ancestor).is_ok() {
                        bail!("dangling profile directory: {}", ancestor.display());
                    }
                }
                Err(error) => return Err(error).context("cannot inspect profile directory"),
            }
        }
        Ok(())
    }

    /// Ask the kernel for the object opened by this descriptor, avoiding a second
    /// canonicalization of the user-controlled source path to establish its identity.
    fn opened_path(file: &File) -> Result<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            let mut bytes = vec![0u8; libc::PATH_MAX as usize];
            if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, bytes.as_mut_ptr()) } == -1 {
                return Err(std::io::Error::last_os_error())
                    .context("cannot locate opened profile");
            }
            bytes.truncate(
                bytes
                    .iter()
                    .position(|b| *b == 0)
                    .context("invalid opened profile path")?,
            );
            Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
        }
        #[cfg(target_os = "linux")]
        {
            fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
                .context("cannot locate opened profile")
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            bail!("profile identity validation requires macOS or Linux")
        }
    }

    impl Source {
        /// Open once, validate fstat metadata, read from that descriptor, then verify
        /// the pathname still names that object. Nonblocking avoids hanging on FIFOs.
        pub fn read(path: &Path, allow_shared: bool) -> Result<(Self, String)> {
            directory(
                path.parent().context("profile has no parent")?,
                allow_shared,
            )?;
            let mut file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open(path)?;
            let metadata = file.metadata()?;
            if !metadata.is_file() {
                bail!("profile must be a regular file: {}", path.display());
            }
            if !allow_shared {
                permissions(path, &metadata, false)?;
            }
            let target = opened_path(&file)?;
            directory(
                target.parent().context("profile target has no parent")?,
                allow_shared,
            )?;
            let stamp = Stamp::from(&metadata);
            let mut text = String::new();
            file.read_to_string(&mut text)
                .context("cannot read profile text")?;
            let source = Self {
                path: path.into(),
                target,
                file,
                stamp,
                allow_shared,
            };
            source.revalidate()?;
            Ok((source, text))
        }

        /// Check descriptor, selected path, and protected target immediately before exec.
        pub fn revalidate(&self) -> Result<()> {
            directory(self.path.parent().unwrap(), self.allow_shared)?;
            directory(self.target.parent().unwrap(), self.allow_shared)?;
            for metadata in [
                self.file.metadata()?,
                fs::metadata(&self.path)?,
                fs::metadata(&self.target)?,
            ] {
                if Stamp::from(&metadata) != self.stamp {
                    bail!("profile changed after loading: {}", self.path.display());
                }
            }
            if opened_path(&self.file)? != self.target
                || fs::canonicalize(&self.path)? != self.target
            {
                bail!(
                    "profile target changed after loading: {}",
                    self.path.display()
                );
            }
            Ok(())
        }
    }
}

#[cfg(not(unix))]
mod platform {
    use anyhow::{Result, bail};
    use std::path::{Path, PathBuf};

    /// No unverified source can be constructed on unsupported platforms.
    #[derive(Debug)]
    pub struct Source {
        /// Placeholder required by the portable policy interface; never populated.
        pub target: PathBuf,
    }

    /// Refuse platforms without the Unix ownership/descriptor guarantees.
    pub fn directory(_path: &Path, _allow_shared: bool) -> Result<()> {
        bail!("profile identity validation is unsupported; refusing to run unsandboxed")
    }

    impl Source {
        /// Unsupported platforms cannot load a policy for execution.
        pub fn read(_path: &Path, _allow_shared: bool) -> Result<(Self, String)> {
            bail!("profile identity validation is unsupported; refusing to run unsandboxed")
        }
        /// No trusted source identity can exist on this platform.
        pub fn revalidate(&self) -> Result<()> {
            bail!("profile identity validation is unsupported; refusing to run unsandboxed")
        }
    }
}

pub use platform::{Source, directory};
