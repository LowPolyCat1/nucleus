use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use nucleus_sandbox::{BindMount, ContainerSpec, ExecSpec, MountMode, NetworkPolicy, SandboxBackend, current_user};
use sha2::{Digest, Sha256};

use crate::{MountKind, TemplateManifest, WORKSPACE_MOUNT};

/// Identity of a build: hash of the manifest, the lockfiles (in order) and the image id.
pub fn identity(manifest: &TemplateManifest, repo_root: &Path, image_id: &str) -> crate::Result<String> {
    let mut h = Sha256::new();
    h.update(toml::to_string(manifest)?.as_bytes());
    for f in &manifest.build.lockfiles {
        let content = std::fs::read(repo_root.join(f)).with_context(|| format!("lockfile {f} is missing"))?;
        h.update(f.as_bytes());
        h.update((content.len() as u64).to_le_bytes());
        h.update(&content);
    }
    h.update(image_id.as_bytes());
    Ok(hex::encode(h.finalize())[..16].to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOutcome {
    pub identity: String,
    /// Host directory with the built template.
    pub path: PathBuf,
    /// False if an existing build with the same identity was reused.
    pub built: bool,
    pub log: String,
}

/// Builds templates inside containers and stores the results under `root/<name>/<identity>`.
pub struct TemplateBuilder<'a> {
    pub backend: &'a dyn SandboxBackend,
    pub root: PathBuf,
}

impl TemplateBuilder<'_> {
    pub fn build_dir(&self, name: &str, identity: &str) -> PathBuf {
        self.root.join(name).join(identity)
    }

    fn marker(&self, name: &str, identity: &str) -> PathBuf {
        self.root.join(name).join(format!("{identity}.done"))
    }

    /// Whether a completed build exists for this identity.
    pub fn is_built(&self, name: &str, identity: &str) -> bool {
        self.marker(name, identity).exists() && self.build_dir(name, identity).is_dir()
    }

    /// Build `manifest` against the repository at `repo_root`, or reuse an existing build.
    /// `default_image` is the agents' base image, used unless the manifest names another.
    pub async fn build(
        &self,
        manifest: &TemplateManifest,
        repo_root: &Path,
        default_image: &str,
    ) -> crate::Result<BuildOutcome> {
        self.build_streaming(manifest, repo_root, default_image, &|_| {}).await
    }

    /// Path of the log of the most recent build of `name`.
    pub fn last_log_path(&self, name: &str) -> PathBuf {
        self.root.join(name).join("last-build.log")
    }

    /// Output of the most recent build of `name`, if any.
    pub fn last_log(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.last_log_path(name)).ok()
    }

    /// Like [`Self::build`], passing every output line to `on_line` as it is produced.
    pub async fn build_streaming(
        &self,
        manifest: &TemplateManifest,
        repo_root: &Path,
        default_image: &str,
        on_line: &(dyn Fn(&str) + Send + Sync),
    ) -> crate::Result<BuildOutcome> {
        let image = manifest
            .build
            .image
            .clone()
            .unwrap_or_else(|| default_image.to_string());
        let image_id = self.backend.ensure_image(&image).await?;
        let id = identity(manifest, repo_root, &image_id)?;
        let out = self.build_dir(&manifest.name, &id);
        if self.is_built(&manifest.name, &id) {
            return Ok(BuildOutcome {
                identity: id,
                path: out,
                built: false,
                log: String::new(),
            });
        }
        if out.exists() {
            tokio::fs::remove_dir_all(&out).await?;
        }
        tokio::fs::create_dir_all(&out).await?;

        // Lockfiles are copied so the build sees a consistent snapshot.
        let src = tempfile::tempdir()?;
        for f in &manifest.build.lockfiles {
            let dest = src.path().join(f);
            if let Some(p) = dest.parent() {
                tokio::fs::create_dir_all(p).await?;
            }
            tokio::fs::copy(repo_root.join(f), &dest).await?;
        }
        let scratch_workspace = tempfile::tempdir()?;
        let mount_path = manifest.mount_path();
        let name = format!(
            "nucleus-build-{}-{}",
            manifest.name,
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        let mut spec = ContainerSpec::new(&name, &image);
        spec.user = current_user();
        spec.labels
            .insert("nucleus.template-build".into(), manifest.name.clone());
        spec.binds.push(BindMount {
            source: src.path().into(),
            target: "/src".into(),
            mode: MountMode::ReadOnly,
        });
        if matches!(manifest.mount, MountKind::Worktree { .. }) {
            spec.binds.push(BindMount {
                source: scratch_workspace.path().into(),
                target: WORKSPACE_MOUNT.into(),
                mode: MountMode::ReadWrite,
            });
        }
        spec.binds.push(BindMount {
            source: out.clone(),
            target: mount_path.clone(),
            mode: MountMode::ReadWrite,
        });
        spec.network = if manifest.build.network.iter().any(|h| h == "*") {
            NetworkPolicy::Full
        } else {
            NetworkPolicy::Allowlist(manifest.build.network.clone())
        };
        spec.env.extend(manifest.env.clone());
        spec.env.insert("HOME".into(), "/tmp".into());
        spec.env.insert("NUCLEUS_TEMPLATE_SRC".into(), "/src".into());
        let path_prefix: Vec<String> = manifest.path_env.get("PATH").cloned().unwrap_or_default();
        let image_path = self
            .backend
            .image_env(&image)
            .await?
            .into_iter()
            .find_map(|e| e.strip_prefix("PATH=").map(str::to_string))
            .unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".into());
        for (k, v) in &manifest.path_env {
            if k != "PATH" {
                spec.env.insert(k.clone(), v.join(":"));
            }
        }
        spec.env.insert(
            "PATH".into(),
            path_prefix
                .into_iter()
                .chain([image_path])
                .collect::<Vec<_>>()
                .join(":"),
        );
        spec.workdir = Some(manifest.build.workdir.clone().unwrap_or(mount_path));

        self.backend.create(&spec).await?;
        // Give a proxy sidecar a moment to start listening before the build reaches out.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let result = self.run_streaming(&name, manifest, on_line).await;
        self.backend.remove(&name).await.ok();
        let (log, exit_code) = result?;
        std::fs::write(self.last_log_path(&manifest.name), &log).ok();
        if exit_code != Some(0) {
            bail!(
                "building template {} failed (exit {:?}):\n{}",
                manifest.name,
                exit_code,
                tail(&log, 4000)
            );
        }
        tokio::fs::write(self.marker(&manifest.name, &id), &image_id).await?;
        Ok(BuildOutcome {
            identity: id,
            path: out,
            built: true,
            log,
        })
    }

    async fn run_streaming(
        &self,
        container: &str,
        manifest: &TemplateManifest,
        on_line: &(dyn Fn(&str) + Send + Sync),
    ) -> crate::Result<(String, Option<i64>)> {
        use futures::StreamExt;
        let spec = ExecSpec::new(["sh", "-ec", manifest.build.command.as_str()]);
        let (mut output, _, exit) = self.backend.exec(container, &spec).await?.into_parts();
        let mut log = String::new();
        let line = |text: String, log: &mut String| {
            on_line(&text);
            log.push_str(&text);
            log.push('\n');
        };
        let (mut out_buf, mut err_buf) = (Vec::new(), Vec::new());
        while let Some(chunk) = output.next().await {
            let (buf, bytes) = match chunk? {
                nucleus_sandbox::ExecChunk::Stdout(b) => (&mut out_buf, b),
                nucleus_sandbox::ExecChunk::Stderr(b) => (&mut err_buf, b),
            };
            buf.extend_from_slice(&bytes);
            while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                let l: Vec<u8> = buf.drain(..=pos).collect();
                line(String::from_utf8_lossy(&l[..l.len() - 1]).into_owned(), &mut log);
            }
        }
        for buf in [out_buf, err_buf] {
            if !buf.is_empty() {
                line(String::from_utf8_lossy(&buf).into_owned(), &mut log);
            }
        }
        Ok((log, exit.await?))
    }

    /// Remove builds of `name` other than `keep`.
    pub async fn prune(&self, name: &str, keep: &[String]) -> crate::Result<()> {
        let dir = self.root.join(name);
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            return Ok(());
        };
        while let Some(e) = entries.next_entry().await? {
            let file = e.file_name().to_string_lossy().to_string();
            if file == "last-build.log" {
                continue;
            }
            let id = file.trim_end_matches(".done");
            if keep.iter().any(|k| k == id) {
                continue;
            }
            if e.file_type().await?.is_dir() {
                // Builds can contain read-only files (e.g. Go module caches).
                make_writable(&e.path());
                tokio::fs::remove_dir_all(e.path()).await?;
            } else {
                tokio::fs::remove_file(e.path()).await?;
            }
        }
        Ok(())
    }
}

fn make_writable(path: &Path) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    let mut perms = meta.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(perms.mode() | 0o700);
    }
    #[cfg(not(unix))]
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    let _ = std::fs::set_permissions(path, perms);
    if meta.is_dir()
        && let Ok(rd) = std::fs::read_dir(path)
    {
        for e in rd.flatten() {
            make_writable(&e.path());
        }
    }
}

fn tail(s: &str, n: usize) -> &str {
    let start = s.len().saturating_sub(n);
    let start = (start..s.len()).find(|&i| s.is_char_boundary(i)).unwrap_or(s.len());
    &s[start..]
}
