//! Shared package manager caches. They are named volumes shared by every container, so
//! installs stay fast across conversations. They are not part of templates.

use std::collections::BTreeMap;

use crate::VolumeMount;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cache {
    pub name: &'static str,
    pub volume: &'static str,
    pub target: &'static str,
    pub env: &'static [(&'static str, &'static str)],
}

pub const DEFAULT_CACHES: &[Cache] = &[
    Cache {
        name: "npm",
        volume: "nucleus-cache-npm",
        target: "/caches/npm",
        env: &[("npm_config_cache", "/caches/npm")],
    },
    Cache {
        name: "pnpm",
        volume: "nucleus-cache-pnpm",
        target: "/caches/pnpm",
        env: &[("npm_config_store_dir", "/caches/pnpm")],
    },
    Cache {
        name: "pip",
        volume: "nucleus-cache-pip",
        target: "/caches/pip",
        env: &[("PIP_CACHE_DIR", "/caches/pip"), ("UV_CACHE_DIR", "/caches/pip/uv")],
    },
    Cache {
        name: "cargo",
        volume: "nucleus-cache-cargo",
        target: "/caches/cargo",
        // Registry, git checkouts and installed binaries. A template setting CARGO_HOME as well
        // is reported as a conflict at workspace creation.
        env: &[("CARGO_HOME", "/caches/cargo")],
    },
];

/// Volume mounts and environment for a set of caches.
pub fn mounts(caches: &[Cache]) -> (Vec<VolumeMount>, BTreeMap<String, String>) {
    let mut env = BTreeMap::new();
    let vols = caches
        .iter()
        .map(|c| {
            env.extend(c.env.iter().map(|(k, v)| (k.to_string(), v.to_string())));
            VolumeMount {
                volume: c.volume.into(),
                target: c.target.into(),
            }
        })
        .collect();
    (vols, env)
}
