//! Settings as the UI sees them: secrets are write-only.

use std::collections::BTreeMap;

use anyhow::bail;
use nucleus_harness::Settings;

/// Placeholder sent to the UI instead of secret values. Must match `SECRET_MASK` in
/// `src/api/backend.ts`.
pub const SECRET_MASK: &str = "••••••••";

/// Settings with every provider secret replaced by the mask.
pub fn masked(settings: &Settings) -> Settings {
    let mut s = settings.clone();
    for v in s.provider_env.values_mut() {
        *v = SECRET_MASK.to_string();
    }
    s
}

/// Apply settings coming from the UI. A masked value keeps the stored secret, an empty value
/// removes it, anything else replaces it.
pub fn merge(current: &Settings, incoming: Settings) -> anyhow::Result<Settings> {
    if incoming.image.trim().is_empty() {
        bail!("image must not be empty");
    }
    let mut env = BTreeMap::new();
    for (k, v) in incoming.provider_env {
        let k = k.trim().to_string();
        if k.is_empty() || k.contains('=') {
            bail!("invalid environment variable name {k:?}");
        }
        if v == SECRET_MASK {
            if let Some(old) = current.provider_env.get(&k) {
                env.insert(k, old.clone());
            }
        } else if !v.is_empty() {
            env.insert(k, v);
        }
    }
    let merged = Settings {
        image: incoming.image.trim().to_string(),
        model: incoming
            .model
            .clone()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty()),
        provider_env: env,
        ..incoming
    };
    merged.validate()?;
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(env: &[(&str, &str)]) -> Settings {
        Settings {
            provider_env: env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn masks_every_secret() {
        let m = masked(&settings(&[("ANTHROPIC_API_KEY", "sk-1")]));
        assert_eq!(m.provider_env["ANTHROPIC_API_KEY"], SECRET_MASK);
    }

    #[test]
    fn merge_keeps_replaces_and_removes() {
        let current = settings(&[("A", "old-a"), ("B", "old-b"), ("C", "old-c")]);
        let incoming = settings(&[
            ("A", SECRET_MASK),
            ("B", "new-b"),
            ("C", ""),
            ("D", SECRET_MASK),
            ("E", "e"),
        ]);
        let out = merge(&current, incoming).unwrap();
        assert_eq!(
            out.provider_env,
            BTreeMap::from([
                ("A".into(), "old-a".into()),
                ("B".into(), "new-b".into()),
                ("E".into(), "e".into())
            ])
        );
    }

    #[test]
    fn merge_validates() {
        let cur = Settings::default();
        assert!(
            merge(
                &cur,
                Settings {
                    image: "  ".into(),
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert!(merge(&cur, settings(&[("A=B", "x")])).is_err());
        assert!(merge(&cur, settings(&[(" ", "x")])).is_err());
        let out = merge(
            &cur,
            Settings {
                model: Some("  ".into()),
                image: " img ".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(out.model, None);
        assert_eq!(out.image, "img");
    }
}
