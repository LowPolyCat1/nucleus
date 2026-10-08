//! Finding the container engine's API endpoint on Linux, macOS and Windows.

use std::path::{Path, PathBuf};

use anyhow::bail;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// Unix domain socket (Linux, macOS).
    Unix(PathBuf),
    /// Windows named pipe, e.g. `//./pipe/docker_engine`.
    NamedPipe(String),
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::Unix(p) => write!(f, "unix://{}", p.display()),
            Endpoint::NamedPipe(p) => write!(f, "npipe://{p}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        }
    }
}

/// Parse an engine address as found in `DOCKER_HOST`: `unix:///path`, `npipe:////./pipe/name`,
/// or a bare socket path. TCP endpoints are rejected: bind mounts refer to local paths, so a
/// remote engine cannot work.
pub fn parse(address: &str) -> anyhow::Result<Endpoint> {
    let a = address.trim();
    if let Some(p) = a.strip_prefix("unix://") {
        return Ok(Endpoint::Unix(PathBuf::from(p)));
    }
    if let Some(p) = a.strip_prefix("npipe://") {
        return Ok(Endpoint::NamedPipe(
            p.trim_start_matches('/')
                .replace('\\', "/")
                .replace("./pipe", "//./pipe"),
        ));
    }
    if a.contains("://") {
        bail!("unsupported engine address {a}: nucleus needs a local engine (unix socket or named pipe)");
    }
    if a.starts_with(r"\\.\pipe\") || a.starts_with("//./pipe/") {
        return Ok(Endpoint::NamedPipe(a.replace('\\', "/")));
    }
    if a.is_empty() {
        bail!("empty engine address");
    }
    Ok(Endpoint::Unix(PathBuf::from(a)))
}

/// Candidate endpoints in priority order: explicit overrides first, then Podman, then Docker
/// and other Docker-compatible engines.
pub fn candidates(os: Os, env: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<Vec<Endpoint>> {
    for var in ["NUCLEUS_CONTAINER_SOCKET", "CONTAINER_HOST", "DOCKER_HOST"] {
        if let Some(v) = env(var).filter(|v| !v.trim().is_empty()) {
            return Ok(vec![parse(&v)?]);
        }
    }
    let home = env("HOME").or_else(|| env("USERPROFILE")).map(PathBuf::from);
    let in_home = |rel: &str| home.as_ref().map(|h| Endpoint::Unix(h.join(rel)));
    let mut out: Vec<Option<Endpoint>> = Vec::new();
    match os {
        Os::Linux => {
            out.push(env("XDG_RUNTIME_DIR").map(|d| Endpoint::Unix(Path::new(&d).join("podman/podman.sock"))));
            out.push(Some(Endpoint::Unix("/run/podman/podman.sock".into())));
            out.push(Some(Endpoint::Unix("/var/run/docker.sock".into())));
            out.push(in_home(".docker/desktop/docker.sock"));
            out.push(in_home(".docker/run/docker.sock"));
        }
        Os::MacOs => {
            out.push(in_home(".local/share/containers/podman/machine/podman.sock"));
            out.push(in_home(
                ".local/share/containers/podman/machine/podman-machine-default/podman.sock",
            ));
            out.push(in_home(".local/share/containers/podman/machine/qemu/podman.sock"));
            out.push(in_home(".local/share/containers/podman/machine/applehv/podman.sock"));
            out.push(Some(Endpoint::Unix("/var/run/docker.sock".into())));
            out.push(in_home(".docker/run/docker.sock"));
            out.push(in_home(".colima/default/docker.sock"));
            out.push(in_home(".orbstack/run/docker.sock"));
            out.push(in_home(".rd/docker.sock"));
        }
        Os::Windows => {
            out.push(Some(Endpoint::NamedPipe("//./pipe/podman-machine-default".into())));
            out.push(Some(Endpoint::NamedPipe("//./pipe/docker_engine".into())));
        }
    }
    Ok(out.into_iter().flatten().collect())
}

fn exists(e: &Endpoint) -> bool {
    match e {
        Endpoint::Unix(p) => p.exists(),
        Endpoint::NamedPipe(p) => Path::new(&p.replace('/', "\\")).exists(),
    }
}

/// The first endpoint that exists on this machine. An explicit override is returned even if
/// it does not exist, so the connection error names it.
pub fn detect() -> anyhow::Result<Endpoint> {
    let env = |k: &str| std::env::var(k).ok();
    let explicit = ["NUCLEUS_CONTAINER_SOCKET", "CONTAINER_HOST", "DOCKER_HOST"]
        .iter()
        .any(|v| env(v).is_some_and(|x| !x.trim().is_empty()));
    let list = candidates(Os::current(), &env)?;
    if explicit {
        return Ok(list.into_iter().next().expect("one explicit endpoint"));
    }
    match list.iter().find(|e| exists(e)) {
        Some(e) => Ok(e.clone()),
        None => bail!(
            "no Podman or Docker engine found (looked at {}); start Podman (`podman machine start` or `systemctl --user start podman.socket`) or Docker, or set NUCLEUS_CONTAINER_SOCKET",
            list.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// How a host path must be written in a bind mount for this engine and OS.
pub fn bind_source(path: &Path, os: Os, podman: bool) -> String {
    let s = path.to_string_lossy().to_string();
    if os != Os::Windows {
        return s;
    }
    let forward = s.trim_start_matches(r"\\?\").replace('\\', "/");
    let bytes = forward.as_bytes();
    if podman && bytes.len() >= 2 && bytes[1] == b':' {
        // Podman machines on Windows run in WSL, where drives live under /mnt.
        format!("/mnt/{}{}", (bytes[0] as char).to_ascii_lowercase(), &forward[2..])
    } else {
        forward
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn parses_addresses() {
        assert_eq!(
            parse("unix:///run/x.sock").unwrap(),
            Endpoint::Unix("/run/x.sock".into())
        );
        assert_eq!(parse("/run/y.sock").unwrap(), Endpoint::Unix("/run/y.sock".into()));
        assert_eq!(
            parse("npipe:////./pipe/docker_engine").unwrap(),
            Endpoint::NamedPipe("//./pipe/docker_engine".into())
        );
        assert_eq!(
            parse(r"\\.\pipe\podman-machine-default").unwrap(),
            Endpoint::NamedPipe("//./pipe/podman-machine-default".into())
        );
        assert!(
            parse("tcp://10.0.0.1:2375")
                .unwrap_err()
                .to_string()
                .contains("local engine")
        );
        assert!(parse("ssh://host").is_err());
        assert!(parse("  ").is_err());
    }

    #[test]
    fn overrides_win_in_order() {
        let c = candidates(
            Os::Linux,
            &env(&[("DOCKER_HOST", "unix:///d.sock"), ("CONTAINER_HOST", "unix:///c.sock")]),
        )
        .unwrap();
        assert_eq!(c, vec![Endpoint::Unix("/c.sock".into())]);
        let c = candidates(
            Os::Linux,
            &env(&[
                ("NUCLEUS_CONTAINER_SOCKET", "/n.sock"),
                ("DOCKER_HOST", "unix:///d.sock"),
            ]),
        )
        .unwrap();
        assert_eq!(c, vec![Endpoint::Unix("/n.sock".into())]);
        assert!(candidates(Os::Linux, &env(&[("DOCKER_HOST", "tcp://x:1")])).is_err());
        // Blank overrides are ignored.
        assert!(candidates(Os::Linux, &env(&[("DOCKER_HOST", " ")])).unwrap().len() > 1);
    }

    #[test]
    fn per_os_defaults() {
        let linux = candidates(
            Os::Linux,
            &env(&[("XDG_RUNTIME_DIR", "/run/user/1000"), ("HOME", "/home/u")]),
        )
        .unwrap();
        assert_eq!(linux[0], Endpoint::Unix("/run/user/1000/podman/podman.sock".into()));
        assert!(linux.contains(&Endpoint::Unix("/var/run/docker.sock".into())));
        // Without XDG_RUNTIME_DIR or HOME the fixed paths remain.
        assert_eq!(candidates(Os::Linux, &env(&[])).unwrap().len(), 2);
        let mac = candidates(Os::MacOs, &env(&[("HOME", "/Users/u")])).unwrap();
        assert_eq!(
            mac[0],
            Endpoint::Unix("/Users/u/.local/share/containers/podman/machine/podman.sock".into())
        );
        assert!(mac.contains(&Endpoint::Unix("/Users/u/.colima/default/docker.sock".into())));
        let win = candidates(Os::Windows, &env(&[("USERPROFILE", r"C:\Users\u")])).unwrap();
        assert_eq!(
            win,
            vec![
                Endpoint::NamedPipe("//./pipe/podman-machine-default".into()),
                Endpoint::NamedPipe("//./pipe/docker_engine".into())
            ]
        );
    }

    #[test]
    fn bind_sources() {
        assert_eq!(bind_source(Path::new("/home/u/x"), Os::Linux, true), "/home/u/x");
        assert_eq!(
            bind_source(Path::new(r"C:\Users\u\x"), Os::Windows, false),
            "C:/Users/u/x"
        );
        assert_eq!(
            bind_source(Path::new(r"C:\Users\u\x"), Os::Windows, true),
            "/mnt/c/Users/u/x"
        );
        assert_eq!(bind_source(Path::new(r"\\?\D:\data"), Os::Windows, true), "/mnt/d/data");
    }

    #[test]
    fn display() {
        assert_eq!(Endpoint::Unix("/a".into()).to_string(), "unix:///a");
        assert_eq!(
            Endpoint::NamedPipe("//./pipe/x".into()).to_string(),
            "npipe:////./pipe/x"
        );
    }
}
