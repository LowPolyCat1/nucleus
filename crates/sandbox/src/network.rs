//! Turning a [`NetworkPolicy`] into concrete container networking.

use crate::NetworkPolicy;

/// How a container's network is wired up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EgressPlan {
    /// `network_mode: none`. Nothing reachable at all.
    Isolated,
    /// Internal network with an egress proxy sidecar enforcing this allowlist.
    Proxied { allow: Vec<String> },
    /// The engine's default network.
    Open,
}

impl EgressPlan {
    pub fn for_policy(policy: &NetworkPolicy, required: &[String]) -> Self {
        let mut allow: Vec<String> = required.iter().map(|h| normalize(h)).collect();
        match policy {
            NetworkPolicy::Full => return EgressPlan::Open,
            NetworkPolicy::None => {}
            NetworkPolicy::Allowlist(hosts) => allow.extend(hosts.iter().map(|h| normalize(h))),
        }
        allow.retain(|h| !h.is_empty());
        allow.sort();
        allow.dedup();
        if allow.iter().any(|h| h == "*") {
            EgressPlan::Open
        } else if allow.is_empty() {
            EgressPlan::Isolated
        } else {
            EgressPlan::Proxied { allow }
        }
    }
}

/// Lower-case a host and strip any scheme, path or port someone may have pasted.
fn normalize(host: &str) -> String {
    let h = host.trim().to_ascii_lowercase();
    let h = h.split("://").last().unwrap_or_default();
    let h = h.split('/').next().unwrap_or_default();
    h.rsplit_once(':')
        .filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit()))
        .map_or(h, |(h, _)| h)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans() {
        let req = vec!["api.anthropic.com".to_string()];
        assert_eq!(
            EgressPlan::for_policy(&NetworkPolicy::None, &[]),
            EgressPlan::Isolated
        );
        assert_eq!(
            EgressPlan::for_policy(&NetworkPolicy::None, &req),
            EgressPlan::Proxied { allow: req.clone() }
        );
        assert_eq!(
            EgressPlan::for_policy(
                &NetworkPolicy::Allowlist(vec![
                    "https://Registry.npmjs.org/".into(),
                    "pypi.org:443".into()
                ]),
                &req
            ),
            EgressPlan::Proxied {
                allow: vec![
                    "api.anthropic.com".into(),
                    "pypi.org".into(),
                    "registry.npmjs.org".into()
                ]
            }
        );
        assert_eq!(
            EgressPlan::for_policy(&NetworkPolicy::Full, &req),
            EgressPlan::Open
        );
    }
}
