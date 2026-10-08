//! Tauri commands. Thin wrappers: argument names are camelCase on the JS side
//! (`src/api/tauri.ts`) and snake_case here.

use std::path::PathBuf;

use nucleus_core::{TranscriptEntry, TurnSummary};
use nucleus_harness::{
    CleanupReport, Conversation, DeleteMode, DeleteOutcome, ProposalDetail, Settings, TemplateStatus, Workspace,
};
use nucleus_promotion::{LibraryKind, Proposal};
use nucleus_sandbox::NetworkPolicy;
use nucleus_skills::SkillSummary;
use nucleus_templates::TemplateManifest;
use nucleus_vcs::{BranchInfo, CommitInfo, FileDiff, MergeOutcome};
use tauri::State;

use crate::core::{AppCore, AppInfo, AppState, CmdResult, err};

type S<'a> = State<'a, AppCore>;

#[tauri::command]
pub async fn init(core: S<'_>) -> CmdResult<AppInfo> {
    Ok(core.init().await)
}

#[tauri::command]
pub async fn get_state(core: S<'_>) -> CmdResult<AppState> {
    core.state().await
}

#[tauri::command]
pub async fn update_settings(core: S<'_>, settings: Settings) -> CmdResult<()> {
    core.update_settings(settings).await
}

#[tauri::command]
pub async fn pick_directory(app: tauri::AppHandle) -> CmdResult<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |path| {
        let _ = tx.send(
            path.and_then(|p| p.into_path().ok())
                .map(|p| p.to_string_lossy().to_string()),
        );
    });
    rx.await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_workspace(core: S<'_>, path: String, name: Option<String>) -> CmdResult<Workspace> {
    core.harness()
        .await?
        .add_workspace(&PathBuf::from(path), name)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn remove_workspace(core: S<'_>, id: String) -> CmdResult<()> {
    core.harness().await?.remove_workspace(&id).await.map_err(err)
}

#[tauri::command]
pub async fn configure_workspace(
    core: S<'_>,
    id: String,
    templates: Vec<String>,
    network: NetworkPolicy,
) -> CmdResult<Workspace> {
    core.harness()
        .await?
        .configure_workspace(&id, templates, network)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn available_templates(core: S<'_>) -> CmdResult<Vec<TemplateManifest>> {
    Ok(core.harness().await?.available_templates())
}

#[tauri::command]
pub async fn template_status(core: S<'_>, workspace_id: String) -> CmdResult<Vec<TemplateStatus>> {
    core.harness().await?.template_status(&workspace_id).await.map_err(err)
}

#[tauri::command]
pub async fn build_templates(core: S<'_>, workspace_id: String) -> CmdResult<()> {
    core.harness()
        .await?
        .build_templates(&workspace_id)
        .await
        .map(drop)
        .map_err(err)
}

#[tauri::command]
pub async fn branches(core: S<'_>, workspace_id: String) -> CmdResult<Vec<BranchInfo>> {
    core.harness().await?.branches(&workspace_id).await.map_err(err)
}

#[tauri::command]
pub async fn graph(core: S<'_>, workspace_id: String, limit: usize) -> CmdResult<Vec<CommitInfo>> {
    core.harness()
        .await?
        .graph(&workspace_id, limit.min(5000))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn diff(core: S<'_>, workspace_id: String, from: String, to: String) -> CmdResult<Vec<FileDiff>> {
    core.harness().await?.diff(&workspace_id, &from, &to).await.map_err(err)
}

#[tauri::command]
pub async fn create_conversation(
    core: S<'_>,
    workspace_id: String,
    base_branch: String,
    title: String,
) -> CmdResult<Conversation> {
    core.harness()
        .await?
        .create_conversation(&workspace_id, &base_branch, &title)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn rename_conversation(core: S<'_>, id: String, title: String) -> CmdResult<()> {
    core.harness()
        .await?
        .rename_conversation(&id, &title)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn delete_conversation(core: S<'_>, id: String, mode: DeleteMode) -> CmdResult<DeleteOutcome> {
    core.harness().await?.delete_conversation(&id, mode).await.map_err(err)
}

#[tauri::command]
pub async fn send_message(core: S<'_>, id: String, prompt: String) -> CmdResult<TurnSummary> {
    core.harness().await?.send_message(&id, &prompt).await.map_err(err)
}

#[tauri::command]
pub async fn cancel(core: S<'_>, id: String) -> CmdResult<()> {
    core.harness().await?.cancel(&id).await.map_err(err)
}

#[tauri::command]
pub async fn transcript(core: S<'_>, id: String) -> CmdResult<Vec<TranscriptEntry>> {
    core.harness().await?.transcript(&id).map_err(err)
}

#[tauri::command]
pub async fn conversation_diff(core: S<'_>, id: String) -> CmdResult<Vec<FileDiff>> {
    core.harness().await?.conversation_diff(&id).await.map_err(err)
}

#[tauri::command]
pub async fn unmerged_commits(core: S<'_>, id: String) -> CmdResult<Vec<CommitInfo>> {
    core.harness().await?.unmerged_commits(&id).await.map_err(err)
}

#[tauri::command]
pub async fn merge_conversation(core: S<'_>, id: String, into: String) -> CmdResult<MergeOutcome> {
    core.harness().await?.merge_conversation(&id, &into).await.map_err(err)
}

#[tauri::command]
pub async fn proposals(core: S<'_>) -> CmdResult<Vec<Proposal>> {
    core.harness().await?.proposals().await.map_err(err)
}

#[tauri::command]
pub async fn proposal(core: S<'_>, kind: LibraryKind, id: String) -> CmdResult<ProposalDetail> {
    core.harness().await?.proposal(kind, &id).await.map_err(err)
}

#[tauri::command]
pub async fn approve(core: S<'_>, kind: LibraryKind, id: String) -> CmdResult<String> {
    core.harness().await?.approve(kind, &id).await.map_err(err)
}

#[tauri::command]
pub async fn reject(core: S<'_>, kind: LibraryKind, id: String) -> CmdResult<()> {
    core.harness().await?.reject(kind, &id).await.map_err(err)
}

#[tauri::command]
pub async fn history(core: S<'_>, kind: LibraryKind, limit: usize) -> CmdResult<Vec<CommitInfo>> {
    core.harness().await?.history(kind, limit).await.map_err(err)
}

#[tauri::command]
pub async fn revert(core: S<'_>, kind: LibraryKind, commit: String) -> CmdResult<String> {
    core.harness().await?.revert(kind, &commit).await.map_err(err)
}

#[tauri::command]
pub async fn skills(core: S<'_>) -> CmdResult<Vec<SkillSummary>> {
    core.harness().await?.skills().list().map_err(err)
}

#[tauri::command]
pub async fn cleanup_orphans(core: S<'_>) -> CmdResult<CleanupReport> {
    core.harness().await?.cleanup_orphans().await.map_err(err)
}

#[tauri::command]
pub async fn build_agent_image(core: S<'_>) -> CmdResult<String> {
    let h = core.harness().await?;
    let image = h.settings().await.image;
    nucleus_harness::build_agent_image(h.backend().engine(), &image)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn recent_logs(
    logs: State<'_, crate::logging::LogBuffer>,
    min_level: Option<String>,
) -> CmdResult<Vec<crate::logging::LogEntry>> {
    Ok(logs.entries(min_level.as_deref()))
}
