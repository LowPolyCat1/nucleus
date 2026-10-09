#![allow(dead_code)]
//! Test fixtures: a git repo, a fake `claude` CLI and a harness on the fake sandbox backend.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nucleus_harness::{Harness, HarnessEvent, Settings};
use nucleus_sandbox::Engine;
use nucleus_sandbox::fake::FakeBackend;
use nucleus_vcs::cli::git;

/// The fake CLI reads its behaviour from `$NUCLEUS_FAKE_DIR/mode` (default `edit`):
/// - `edit`: writes notes.txt, uses a skill, prints a full stream-json turn
/// - `propose`: also drops skill, tool, template and broken proposals in the outbox
/// - `slow`: prints init, then sleeps (for cancel and concurrency tests)
/// - `crash`: prints init and exits 2 without a result
/// - `noop`: completes a turn without touching files
/// - `gitcommit` / `gitlog` / `gitamend`: use git inside the sandbox
pub const FAKE_CLAUDE: &str = r#"#!/bin/sh
mode=$(cat "$NUCLEUS_FAKE_DIR/mode" 2>/dev/null || echo edit)
printf '%s\n' "$@" > "$NUCLEUS_FAKE_DIR/last-args"
echo '{"type":"system","subtype":"init","session_id":"sess-42","tools":["Bash","Skill"],"model":"fake"}'
case "$mode" in
  slow) exec sleep 30 ;;
  crash) echo "boom" >&2; exit 2 ;;
  gitcommit)
    # The agent uses git itself (in the real sandbox /workspace/.git points at $NUCLEUS_GIT_DIR).
    echo committed > committed.txt
    GIT_DIR="$NUCLEUS_GIT_DIR" GIT_WORK_TREE=. git add committed.txt
    GIT_DIR="$NUCLEUS_GIT_DIR" GIT_WORK_TREE=. git commit -q -m "agent: add committed.txt"
    echo loose > loose.txt
    ;;
  gitlog)
    GIT_DIR="$NUCLEUS_GIT_DIR" GIT_WORK_TREE=. git log --format=%s > "$NUCLEUS_FAKE_DIR/gitlog"
    GIT_DIR="$NUCLEUS_GIT_DIR" GIT_WORK_TREE=. git status --porcelain > "$NUCLEUS_FAKE_DIR/gitstatus"
    ;;
  resolve)
    # Resolve every conflicted file by replacing it.
    grep -rl '^<<<<<<<' --exclude-dir=.git . | while read -r f; do echo resolved > "$f"; done
    ;;
  gitamend)
    echo amended >> committed.txt
    GIT_DIR="$NUCLEUS_GIT_DIR" GIT_WORK_TREE=. git commit -q -a --amend -m "agent: rewritten"
    ;;
esac
if [ "$mode" = edit ] || [ "$mode" = propose ]; then
  echo "turn" >> notes.txt
  echo '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t0","name":"Skill","input":{"skill":"rust-tests"}}]}}'
fi
if [ "$mode" = propose ]; then
  mkdir -p "$NUCLEUS_OUTBOX_DIR/proposals" "$NUCLEUS_OUTBOX_DIR/tools/greet" "$NUCLEUS_OUTBOX_DIR/tools/broken"
  printf 'name = "greet"\ndescription = "Greets"\nrun = "echo hello"\ntest = "echo hello | grep -q hello"\n' > "$NUCLEUS_OUTBOX_DIR/tools/greet/tool.toml"
  printf 'name = "broken"\ndescription = "Fails"\nrun = "true"\ntest = "exit 1"\n' > "$NUCLEUS_OUTBOX_DIR/tools/broken/tool.toml"
  printf '%s\n' '{"kind":"skill","content":"---\nname: rust-tests\ndescription: Run rust tests\n---\nUse cargo test.\n","rationale":"worked"}' > "$NUCLEUS_OUTBOX_DIR/proposals/1.json"
  echo '{"kind":"tool","name":"greet","rationale":"handy"}' > "$NUCLEUS_OUTBOX_DIR/proposals/2.json"
  echo '{"kind":"tool","name":"broken","rationale":"x"}' > "$NUCLEUS_OUTBOX_DIR/proposals/3.json"
  printf '{"kind":"template","manifest":"name = \\"py\\"\\nmount = { mode = \\"readonly\\" }\\n[build]\\ncommand = \\"true\\"\\n","rationale":"deps"}\n' > "$NUCLEUS_OUTBOX_DIR/proposals/4.json"
  echo '{"kind":"skill","content":"no frontmatter","rationale":"bad"}' > "$NUCLEUS_OUTBOX_DIR/proposals/5.json"
  echo 'not json' > "$NUCLEUS_OUTBOX_DIR/proposals/6.json"
fi
echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Done"}}}'
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"Done"}]}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"Done","session_id":"sess-42","total_cost_usd":0.01}'
"#;

pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub repo: PathBuf,
    pub fake_dir: PathBuf,
    pub backend: Arc<FakeBackend>,
    pub harness: Harness,
    pub events: Arc<Mutex<Vec<HarnessEvent>>>,
}

pub async fn commit_file(dir: &Path, file: &str, content: &str, msg: &str) {
    if let Some(p) = dir.join(file).parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(dir.join(file), content).unwrap();
    git(dir, &["add", "-A"]).await.unwrap();
    git(
        dir,
        &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", msg],
    )
    .await
    .unwrap();
}

pub async fn fixture(engine: Engine) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]).await.unwrap();
    commit_file(&repo, "README.md", "hello\n", "initial").await;

    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("claude"), FAKE_CLAUDE).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(bin.join("claude"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let fake_dir = dir.path().join("fake");
    std::fs::create_dir_all(&fake_dir).unwrap();

    let backend = Arc::new(FakeBackend::new(engine, dir.path().join("volumes")).with_bin_dir(&bin));
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink_events = events.clone();
    let harness = Harness::open(
        dir.path().join("data"),
        backend.clone(),
        Arc::new(move |e| sink_events.lock().unwrap().push(e)),
    )
    .await
    .unwrap();
    let mut settings = Settings::default();
    settings
        .provider_env
        .insert("ANTHROPIC_API_KEY".into(), "test-key".into());
    settings
        .provider_env
        .insert("NUCLEUS_FAKE_DIR".into(), fake_dir.to_string_lossy().into());
    harness.update_settings(settings).await.unwrap();
    Fixture {
        dir,
        repo,
        fake_dir,
        backend,
        harness,
        events,
    }
}

impl Fixture {
    pub fn mode(&self, mode: &str) {
        std::fs::write(self.fake_dir.join("mode"), mode).unwrap();
    }

    pub fn last_args(&self) -> Vec<String> {
        std::fs::read_to_string(self.fake_dir.join("last-args"))
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    pub fn events(&self) -> Vec<HarnessEvent> {
        self.events.lock().unwrap().clone()
    }

    pub async fn branches(&self) -> Vec<String> {
        nucleus_vcs::GixVcs::open(&self.repo)
            .unwrap()
            .branches()
            .await
            .unwrap()
            .into_iter()
            .map(|b| b.name)
            .collect()
    }
}

use nucleus_vcs::Vcs;
