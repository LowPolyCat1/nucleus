//! Unified diff rendering for blob pairs.

use similar::TextDiff;

pub(crate) struct Rendered {
    pub binary: bool,
    /// The patch was cut short (or skipped for a very large file).
    pub truncated: bool,
    pub additions: usize,
    pub deletions: usize,
    pub patch: String,
}

/// Files larger than this are not diffed at all.
const MAX_DIFF_BYTES: usize = 8 * 1024 * 1024;
/// Patches longer than this are cut at a line boundary.
pub(crate) const MAX_PATCH_BYTES: usize = 1024 * 1024;

pub(crate) fn render(old: &[u8], new: &[u8]) -> Rendered {
    let is_binary = |b: &[u8]| b.iter().take(8000).any(|&c| c == 0);
    if is_binary(old) || is_binary(new) {
        return Rendered {
            binary: true,
            truncated: false,
            additions: 0,
            deletions: 0,
            patch: String::new(),
        };
    }
    if old.len() > MAX_DIFF_BYTES || new.len() > MAX_DIFF_BYTES {
        return Rendered {
            binary: false,
            truncated: true,
            additions: 0,
            deletions: 0,
            patch: String::new(),
        };
    }
    let (old, new) = (String::from_utf8_lossy(old), String::from_utf8_lossy(new));
    let diff = TextDiff::from_lines(old.as_ref(), new.as_ref());
    let (mut additions, mut deletions) = (0, 0);
    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Insert => additions += 1,
            similar::ChangeTag::Delete => deletions += 1,
            similar::ChangeTag::Equal => {}
        }
    }
    let mut patch = diff.unified_diff().context_radius(3).to_string();
    let mut truncated = false;
    if patch.len() > MAX_PATCH_BYTES {
        let cut = patch[..MAX_PATCH_BYTES].rfind('\n').map_or(0, |i| i + 1);
        patch.truncate(cut);
        truncated = true;
    }
    Rendered {
        binary: false,
        truncated,
        additions,
        deletions,
        patch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits() {
        let r = render(b"a\n", b"b\n");
        assert!(!r.truncated && !r.binary);
        assert_eq!((r.additions, r.deletions), (1, 1));
        assert!(render(b"a\0", b"b").binary);
        let huge = vec![b'x'; MAX_DIFF_BYTES + 1];
        let r = render(b"", &huge);
        assert!(r.truncated && r.patch.is_empty() && !r.binary);
        let many: String = (0..200_000).map(|i| format!("line {i}\n")).collect();
        let r = render(b"", many.as_bytes());
        assert!(r.truncated);
        assert!(r.patch.len() <= MAX_PATCH_BYTES && r.patch.ends_with('\n'));
        assert_eq!(r.additions, 200_000, "counts cover the whole file");
    }
}
