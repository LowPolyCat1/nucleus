//! Combining several templates for one workspace.
//!
//! - Each template has its own mount path, so `/deps/*` paths never collide.
//! - List variables are concatenated in template order (first template wins on binary name
//!   clashes), followed by the base value from the image.
//! - Two templates setting the same plain variable, or claiming overlapping in-worktree paths,
//!   is an error, never a silent override. Variables owned by the harness (e.g. cache
//!   locations) count as already set.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::bail;
use nucleus_sandbox::{BindMount, MountMode};

use crate::{MountKind, TemplateManifest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateMount {
    pub manifest: TemplateManifest,
    /// Host directory holding the built template.
    pub built: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedTemplates {
    pub binds: Vec<BindMount>,
    pub env: BTreeMap<String, String>,
    /// `info/exclude` patterns for in-worktree mounts, anchored at the worktree root.
    pub excludes: Vec<String>,
}

const DEFAULT_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Combine `templates` in order. `base_env` is the image environment (for list variables);
/// `reserved` maps variables already owned by someone else to the owner's name.
pub fn resolve(
    templates: &[TemplateMount],
    base_env: &BTreeMap<String, String>,
    reserved: &BTreeMap<String, String>,
    overlay_supported: bool,
) -> crate::Result<ResolvedTemplates> {
    let mut out = ResolvedTemplates::default();
    let mut owners: BTreeMap<String, String> = reserved.clone();
    let mut lists: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut worktree_paths: Vec<(String, String)> = Vec::new();
    let mut names = std::collections::BTreeSet::new();

    for t in templates {
        let m = &t.manifest;
        if !names.insert(m.name.clone()) {
            bail!("template {} is listed twice", m.name);
        }
        let mode = match &m.mount {
            MountKind::Readonly => MountMode::ReadOnly,
            MountKind::Overlay if overlay_supported => MountMode::Overlay,
            MountKind::Overlay => {
                bail!(
                    "template {} needs an overlay mount, which requires Podman",
                    m.name
                )
            }
            MountKind::Worktree { path } => {
                let path = path.trim_end_matches('/').to_string();
                for (other, other_path) in &worktree_paths {
                    let nested = |a: &str, b: &str| a == b || a.starts_with(&format!("{b}/"));
                    if nested(&path, other_path) || nested(other_path, &path) {
                        bail!(
                            "templates {other} and {} both claim worktree path {path}",
                            m.name
                        );
                    }
                }
                worktree_paths.push((m.name.clone(), path.clone()));
                out.excludes.push(format!("/{path}"));
                // Tools that need the directory inside the worktree often write caches into it.
                if overlay_supported {
                    MountMode::Overlay
                } else {
                    MountMode::ReadOnly
                }
            }
        };
        out.binds.push(BindMount {
            source: t.built.clone(),
            target: m.mount_path(),
            mode,
        });
        for (k, v) in &m.env {
            if let Some(owner) = owners.get(k) {
                bail!("templates {owner} and {} both set {k}", m.name);
            }
            if lists.contains_key(k) {
                bail!(
                    "{k} is a list variable in another template but a plain variable in {}",
                    m.name
                );
            }
            owners.insert(k.clone(), m.name.clone());
            out.env.insert(k.clone(), v.clone());
        }
        for (k, v) in &m.path_env {
            if let Some(owner) = owners.get(k) {
                bail!(
                    "{k} is set as a plain variable by {owner} but used as a list by {}",
                    m.name
                );
            }
            lists
                .entry(k.clone())
                .or_default()
                .extend(v.iter().cloned());
        }
    }
    for (k, mut entries) in lists {
        match base_env.get(&k) {
            Some(base) if !base.is_empty() => entries.push(base.clone()),
            None if k == "PATH" => entries.push(DEFAULT_PATH.into()),
            _ => {}
        }
        out.env.insert(k, entries.join(":"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(toml_text: &str) -> TemplateMount {
        let manifest = TemplateManifest::parse(toml_text).unwrap();
        TemplateMount {
            built: format!("/builds/{}", manifest.name).into(),
            manifest,
        }
    }

    #[test]
    fn combines_in_order() {
        let a = t(
            "name='a'\nmount={mode='readonly'}\npath_env={PATH=['/deps/a/bin']}\nenv={A_HOME='/deps/a'}\n[build]\ncommand='x'",
        );
        let b = t(
            "name='b'\nmount={mode='worktree',path='node_modules'}\npath_env={PATH=['/workspace/node_modules/.bin']}\n[build]\ncommand='x'",
        );
        let base = BTreeMap::from([("PATH".to_string(), "/usr/bin".to_string())]);
        let r = resolve(&[a, b], &base, &BTreeMap::new(), false).unwrap();
        assert_eq!(
            r.env["PATH"],
            "/deps/a/bin:/workspace/node_modules/.bin:/usr/bin"
        );
        assert_eq!(r.env["A_HOME"], "/deps/a");
        assert_eq!(r.excludes, vec!["/node_modules"]);
        assert_eq!(r.binds[0].target, "/deps/a");
        assert_eq!(r.binds[1].target, "/workspace/node_modules");
    }

    #[test]
    fn conflicts_are_errors() {
        let a = t("name='a'\nmount={mode='readonly'}\nenv={X='1'}\n[build]\ncommand='x'");
        let b = t("name='b'\nmount={mode='readonly'}\nenv={X='2'}\n[build]\ncommand='x'");
        let e = resolve(&[a.clone(), b], &BTreeMap::new(), &BTreeMap::new(), false).unwrap_err();
        assert!(e.to_string().contains("both set X"));

        let reserved = BTreeMap::from([("X".to_string(), "cache cargo".to_string())]);
        assert!(resolve(std::slice::from_ref(&a), &BTreeMap::new(), &reserved, false).is_err());

        let w1 = t("name='w1'\nmount={mode='worktree',path='vendor'}\n[build]\ncommand='x'");
        let w2 = t("name='w2'\nmount={mode='worktree',path='vendor/sub'}\n[build]\ncommand='x'");
        assert!(resolve(&[w1, w2], &BTreeMap::new(), &BTreeMap::new(), false).is_err());

        let o = t("name='o'\nmount={mode='overlay'}\n[build]\ncommand='x'");
        assert!(
            resolve(
                std::slice::from_ref(&o),
                &BTreeMap::new(),
                &BTreeMap::new(),
                false
            )
            .is_err()
        );
        assert_eq!(
            resolve(&[o], &BTreeMap::new(), &BTreeMap::new(), true)
                .unwrap()
                .binds[0]
                .mode,
            MountMode::Overlay
        );
    }
}
