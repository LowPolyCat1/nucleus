//! Managing `info/exclude`, used for paths mounted inside a worktree that must never be
//! committed. Unlike `.gitignore` this never shows up as a change. `info/` lives in the common
//! git directory, so entries apply to every worktree of the repository.

use std::path::Path;

const BEGIN: &str = "# >>> nucleus managed";
const END: &str = "# <<< nucleus managed";

/// Ensure `patterns` are present in the managed block of `<common_dir>/info/exclude`.
/// Existing managed entries are kept; the operation is idempotent.
pub fn ensure_excluded(common_dir: &Path, patterns: &[String]) -> std::io::Result<()> {
    let path = common_dir.join("info").join("exclude");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let (mut before, mut managed, mut after) = (Vec::new(), Vec::new(), Vec::new());
    let mut section = 0;
    for line in existing.lines() {
        match (section, line) {
            (0, BEGIN) => section = 1,
            (1, END) => section = 2,
            (0, l) => before.push(l.to_string()),
            (1, l) => managed.push(l.to_string()),
            (_, l) => after.push(l.to_string()),
        }
    }
    let mut changed = false;
    for p in patterns {
        if !managed.iter().any(|m| m == p) {
            managed.push(p.clone());
            changed = true;
        }
    }
    if !changed && section == 2 {
        return Ok(());
    }
    let mut out = before;
    out.push(BEGIN.into());
    out.extend(managed);
    out.push(END.into());
    out.extend(after);
    std::fs::create_dir_all(path.parent().expect("has parent"))?;
    std::fs::write(&path, out.join("\n") + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idempotent_and_preserves_user_lines() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("info")).unwrap();
        std::fs::write(dir.path().join("info/exclude"), "*.log\n").unwrap();
        ensure_excluded(dir.path(), &["/node_modules".into()]).unwrap();
        ensure_excluded(dir.path(), &["/node_modules".into(), "/.venv".into()]).unwrap();
        let s = std::fs::read_to_string(dir.path().join("info/exclude")).unwrap();
        assert_eq!(
            s,
            "*.log\n# >>> nucleus managed\n/node_modules\n/.venv\n# <<< nucleus managed\n"
        );
    }
}
