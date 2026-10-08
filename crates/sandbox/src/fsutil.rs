//! Reading files the sandbox can write, without letting it point the harness elsewhere.

use std::io::Read;
use std::path::Path;

/// Read a file that a sandboxed process may have created. Refuses symlinks at the final path
/// component (checked by the OS on unix, so there is no check-then-open race) and anything
/// that is not a regular file, and caps the size.
pub fn read_nofollow(path: &Path, max_bytes: u64) -> std::io::Result<Vec<u8>> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(not(unix))]
    {
        if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(std::io::Error::other(format!("{} is a symlink", path.display())));
        }
    }
    let file = opts.open(path).map_err(|e| {
        if e.raw_os_error() == Some(eloop()) {
            std::io::Error::other(format!("{} is a symlink", path.display()))
        } else {
            e
        }
    })?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(std::io::Error::other(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    if meta.len() > max_bytes {
        return Err(std::io::Error::other(format!(
            "{} is larger than {max_bytes} bytes",
            path.display()
        )));
    }
    let mut buf = Vec::with_capacity(meta.len() as usize);
    file.take(max_bytes + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > max_bytes {
        return Err(std::io::Error::other(format!(
            "{} is larger than {max_bytes} bytes",
            path.display()
        )));
    }
    Ok(buf)
}

/// Like [`read_nofollow`] for UTF-8 text.
pub fn read_text_nofollow(path: &Path, max_bytes: u64) -> std::io::Result<String> {
    String::from_utf8(read_nofollow(path, max_bytes)?)
        .map_err(|_| std::io::Error::other(format!("{} is not UTF-8 text", path.display())))
}

#[cfg(unix)]
fn eloop() -> i32 {
    libc::ELOOP
}

#[cfg(not(unix))]
fn eloop() -> i32 {
    -1
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn refuses_symlinks_dirs_fifos_and_big_files() {
        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("secret");
        std::fs::write(&secret, "host secret").unwrap();
        let ok = dir.path().join("ok.json");
        std::fs::write(&ok, "{}").unwrap();
        assert_eq!(read_text_nofollow(&ok, 10).unwrap(), "{}");
        let link = dir.path().join("link.json");
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        let e = read_nofollow(&link, 100).unwrap_err().to_string();
        assert!(e.contains("symlink"), "{e}");
        assert!(read_nofollow(dir.path(), 100).is_err());
        let fifo = dir.path().join("fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            read_nofollow(&fifo, 100)
                .unwrap_err()
                .to_string()
                .contains("regular file")
        );
        assert!(read_nofollow(&secret, 3).unwrap_err().to_string().contains("larger"));
        std::fs::write(dir.path().join("bin"), [0xff, 0xfe]).unwrap();
        assert!(read_text_nofollow(&dir.path().join("bin"), 10).is_err());
        assert!(read_nofollow(&dir.path().join("missing"), 10).is_err());
    }
}

/// A directory the sandbox can write (outbox, tool candidates). Every access resolves inside
/// it: symlinks and `..` that lead outside are refused by the OS-level capability checks of
/// `cap-std`, so a sandboxed process cannot redirect the harness to host files, even by
/// swapping paths while the harness reads.
pub struct Confined {
    dir: cap_std::fs::Dir,
}

/// One entry of a confined directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfinedEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_file: bool,
    pub is_symlink: bool,
}

impl Confined {
    /// Open `root`, which the harness created and the sandbox cannot replace (a mount point).
    pub fn open(root: &Path) -> std::io::Result<Self> {
        Ok(Self {
            dir: cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())?,
        })
    }

    /// A confined view of a subdirectory.
    pub fn sub(&self, rel: &str) -> std::io::Result<Self> {
        Ok(Self {
            dir: self.dir.open_dir(rel)?,
        })
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.dir.symlink_metadata(rel).is_ok()
    }

    /// Read a regular file, refusing symlinks, special files and files over `max_bytes`.
    pub fn read(&self, rel: &str, max_bytes: u64) -> std::io::Result<Vec<u8>> {
        let meta = self.dir.symlink_metadata(rel)?;
        if meta.file_type().is_symlink() {
            return Err(std::io::Error::other(format!("{rel} is a symlink")));
        }
        if !meta.is_file() {
            return Err(std::io::Error::other(format!("{rel} is not a regular file")));
        }
        let mut opts = cap_std::fs::OpenOptions::new();
        opts.read(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = self.dir.open_with(rel, &opts)?;
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(std::io::Error::other(format!("{rel} is not a regular file")));
        }
        if meta.len() > max_bytes {
            return Err(std::io::Error::other(format!("{rel} is larger than {max_bytes} bytes")));
        }
        let mut buf = Vec::new();
        file.take(max_bytes + 1).read_to_end(&mut buf)?;
        if buf.len() as u64 > max_bytes {
            return Err(std::io::Error::other(format!("{rel} is larger than {max_bytes} bytes")));
        }
        Ok(buf)
    }

    pub fn read_text(&self, rel: &str, max_bytes: u64) -> std::io::Result<String> {
        String::from_utf8(self.read(rel, max_bytes)?)
            .map_err(|_| std::io::Error::other(format!("{rel} is not UTF-8 text")))
    }

    /// Entries of a subdirectory (`""` for the root), sorted by name.
    pub fn list(&self, rel: &str) -> std::io::Result<Vec<ConfinedEntry>> {
        let dir = if rel.is_empty() || rel == "." {
            self.dir.try_clone()?
        } else {
            self.dir.open_dir(rel)?
        };
        let mut out = Vec::new();
        for e in dir.entries()? {
            let e = e?;
            let ft = e.file_type()?;
            out.push(ConfinedEntry {
                name: e.file_name().to_string_lossy().into_owned(),
                is_dir: ft.is_dir(),
                is_file: ft.is_file(),
                is_symlink: ft.is_symlink(),
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Move a file within the directory (rename never follows a symlink at the source).
    pub fn rename(&self, from: &str, to: &str) -> std::io::Result<()> {
        self.dir.rename(from, &self.dir, to)
    }

    /// Whether a regular file has any executable bit set (always false off unix).
    pub fn is_executable(&self, rel: &str) -> bool {
        #[cfg(unix)]
        {
            use cap_std::fs::PermissionsExt;
            self.dir
                .symlink_metadata(rel)
                .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            let _ = rel;
            false
        }
    }

    pub fn create_dir_all(&self, rel: &str) -> std::io::Result<()> {
        self.dir.create_dir_all(rel)
    }

    pub fn remove_file(&self, rel: &str) -> std::io::Result<()> {
        self.dir.remove_file(rel)
    }
}

#[cfg(all(test, unix))]
mod confined_tests {
    use super::*;

    #[test]
    fn stays_inside_its_root() {
        let host = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(host.path().join("secrets")).unwrap();
        std::fs::write(host.path().join("secrets/id_rsa"), "PRIVATE").unwrap();
        let root = host.path().join("outbox");
        std::fs::create_dir_all(root.join("tools/good")).unwrap();
        std::fs::write(root.join("tools/good/tool.toml"), "ok").unwrap();
        // The sandbox points a tool directory and a file at host secrets.
        std::os::unix::fs::symlink(host.path().join("secrets"), root.join("tools/evil")).unwrap();
        std::os::unix::fs::symlink(host.path().join("secrets/id_rsa"), root.join("tools/good/key")).unwrap();
        std::os::unix::fs::symlink("../../secrets/id_rsa", root.join("rel")).unwrap();

        let c = Confined::open(&root).unwrap();
        assert_eq!(c.read_text("tools/good/tool.toml", 10).unwrap(), "ok");
        assert!(c.sub("tools/evil").is_err(), "symlinked directory leading outside");
        assert!(c.read("tools/evil/id_rsa", 100).is_err());
        assert!(
            c.read("tools/good/key", 100)
                .unwrap_err()
                .to_string()
                .contains("symlink")
        );
        assert!(c.read("rel", 100).is_err());
        assert!(c.read("../secrets/id_rsa", 100).is_err());
        assert!(c.read("/etc/passwd", 100).is_err());
        let entries = c.list("tools/good").unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|e| (e.name.as_str(), e.is_symlink))
                .collect::<Vec<_>>(),
            [("key", true), ("tool.toml", false)]
        );
        let sub = c.sub("tools/good").unwrap();
        assert_eq!(sub.read_text("tool.toml", 10).unwrap(), "ok");
        assert!(sub.read("../evil/id_rsa", 100).is_err());
        c.create_dir_all("processed").unwrap();
        c.rename("rel", "processed/rel").unwrap();
        assert!(
            std::fs::symlink_metadata(root.join("processed/rel"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_to_string(host.path().join("secrets/id_rsa")).unwrap(),
            "PRIVATE"
        );
    }
}
