//! Unified diff rendering for blob pairs.

use similar::TextDiff;

pub(crate) struct Rendered {
    pub binary: bool,
    pub additions: usize,
    pub deletions: usize,
    pub patch: String,
}

const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;

pub(crate) fn render(old: &[u8], new: &[u8]) -> Rendered {
    let is_binary = |b: &[u8]| b.len() > MAX_DIFF_BYTES || b.iter().take(8000).any(|&c| c == 0);
    if is_binary(old) || is_binary(new) {
        return Rendered {
            binary: true,
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
    let patch = diff.unified_diff().context_radius(3).to_string();
    Rendered {
        binary: false,
        additions,
        deletions,
        patch,
    }
}
