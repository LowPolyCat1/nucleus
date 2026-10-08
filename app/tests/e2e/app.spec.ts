import { expect, open, setApiKey, test } from "./fixtures";

test("engine error screen and retry", async ({ app }) => {
  await open(app, "delay=0&initError=no%20container%20engine");
  await expect(app.getByTestId("init-error")).toContainText("no container engine");
  await app.evaluate(() => window.__nucleusMock!.setInitError(null));
  await app.getByTestId("retry-init").click();
  await expect(app.getByTestId("workspace-view")).toBeVisible();
});

test("full agent workflow: workspace to merged work", async ({ app }) => {
  await open(app, "delay=0&seed=0");
  await setApiKey(app);

  // Add a workspace.
  await app.getByTestId("add-workspace").click();
  await app.getByLabel("Repository path").fill("/home/me/project");
  await app.getByRole("button", { name: "Add", exact: true }).click();
  await expect(app.getByRole("heading", { name: "project" })).toBeVisible();

  // Start a conversation and chat.
  await app.getByLabel("Conversation title").fill("Write notes");
  await app.getByTestId("start-conversation").click();
  await expect(app.getByTestId("conversation-view")).toBeVisible();
  await app.getByLabel("Message").fill("Add a notes file");
  await app.getByLabel("Message").press("Control+Enter");
  await expect(app.getByTestId("msg-turn")).toContainText("turn complete");
  await expect(app.getByTestId("msg-assistant").last()).toContainText("Done. I updated notes.md");

  // Review and merge.
  await app.getByRole("tab", { name: "Changes" }).click();
  await expect(app.getByTestId("diff-file-notes.md")).toContainText("+- Add a notes file");
  await app.getByTestId("merge").click();
  await expect(app.getByTestId("toast-success").filter({ hasText: "Merged into main" })).toBeVisible();
  await expect(app.getByTestId("unmerged-commits")).toHaveCount(0);

  // Merged work deletes without confirmation.
  await app.getByTestId("delete-conversation").click();
  await app.getByTestId("confirm-delete").click();
  await expect(app.getByTestId("delete-dialog")).toBeHidden();
  await expect(app.getByTestId("workspace-view")).toBeVisible();

  // History shows the agent commit on main.
  await expect(app.getByTestId("graph-row").filter({ hasText: "Agent turn: Add a notes file" })).toBeVisible();
});

test("streaming is visible while the turn runs, and stop works", async ({ app }) => {
  await open(app, "delay=40");
  await setApiKey(app);
  await app.locator('[data-testid^="conversation-"]').first().click();
  await app.getByLabel("Message").fill("slow job");
  await app.getByTestId("send").click();
  await expect(app.getByTestId("stop")).toBeVisible();
  await expect(app.locator('[data-testid="msg-assistant"][data-streaming="true"]')).toBeVisible();
  await expect(app.getByTestId("status-dot").first()).toHaveAttribute("data-status", "running");
  await app.getByTestId("stop").click();
  await expect(app.getByTestId("send")).toBeVisible();
  await expect(app.getByTestId("msg-error").last()).toContainText("exited with code 130");
});

test("deleting unmerged work requires a decision", async ({ app }) => {
  await open(app);
  await app.locator('[data-testid^="conversation-"]').first().click();
  await app.getByTestId("delete-conversation").click();
  await app.getByTestId("confirm-delete").click();
  await expect(app.getByTestId("unmerged-warning")).toContainText("only on this agent branch");
  await app.getByLabel("Copy branch name").fill("local/saved-work");
  await app.getByTestId("keep-copy").click();
  await expect(app.getByTestId("delete-dialog")).toBeHidden();
  await expect(app.getByTestId("branch-local/saved-work")).toBeVisible();
});

test("proposals review", async ({ app }) => {
  await open(app);
  await expect(app.getByTestId("proposal-count")).toHaveText("2");
  await app.getByTestId("nav-proposals").click();
  await expect(app.getByTestId("rationale")).toBeVisible();
  await app.getByTestId("approve").click();
  await expect(app.getByTestId("proposal-count")).toHaveText("1");
  await app.getByTestId("reject").click();
  await expect(app.getByText("No pending proposals")).toBeVisible();
  await app.getByTestId("nav-skills").click();
  await expect(app.getByTestId("library-history")).toBeVisible();
});

test("workspace setup validates templates and hosts", async ({ app }) => {
  await open(app);
  await app.getByRole("tab", { name: "Templates & network" }).click();
  await app.getByLabel("Use template python", { exact: true }).check();
  await app.getByLabel("Use template python-alt").check();
  await app.getByTestId("save-workspace").click();
  await expect(app.getByTestId("workspace-error")).toContainText("both set VIRTUAL_ENV");
  await app.getByLabel("Use template python-alt").uncheck();
  await app.getByRole("radio", { name: /^Allowlist/ }).check();
  await app.getByLabel("Allowed hosts").fill("pypi.org\nnot a host!");
  await expect(app.getByTestId("invalid-hosts")).toBeVisible();
  await expect(app.getByTestId("save-workspace")).toBeDisabled();
  await app.getByLabel("Allowed hosts").fill("pypi.org");
  await app.getByTestId("save-workspace").click();
  await expect(app.getByText("Allowlist: pypi.org")).toBeVisible();
  await app.getByTestId("build-templates").click();
  await expect(app.getByTestId("template-status").getByText("built")).toBeVisible();
});

test("backend errors surface as toasts without breaking the app", async ({ app }) => {
  await open(app);
  await setApiKey(app);
  await app.evaluate(() => window.__nucleusMock!.failNext("createConversation", "engine went away"));
  await app.getByTestId("workspace-demo").click();
  await app.getByTestId("start-conversation").click();
  await expect(app.getByTestId("toast-error").filter({ hasText: "engine went away" })).toBeVisible();
  await app.getByTestId("start-conversation").click();
  await expect(app.getByTestId("conversation-view")).toBeVisible();
});

test("logs view records operations", async ({ app }) => {
  await open(app);
  await app.getByTestId("start-conversation").click();
  await expect(app.getByTestId("conversation-view")).toBeVisible();
  await app.getByTestId("nav-logs").click();
  await expect(app.getByTestId("log-row").filter({ hasText: "conversation created" })).toBeVisible();
});

test("hand-written template goes through review and becomes usable", async ({ app }) => {
  await open(app);
  await app.getByTestId("nav-skills").click();
  await app.getByTestId("new-template").click();
  await app.getByLabel("template.toml").fill('name = "rust"\ndescription = "cargo registry"\nmount = { mode = "readonly" }\n[build]\ncommand = "true"\n');
  await app.getByTestId("submit-proposal").click();
  await expect(app.getByTestId("library-editor")).toBeHidden();
  await app.getByTestId("nav-proposals").click();
  await app.getByRole("button", { name: /Add template rust/ }).click();
  await app.getByTestId("approve").click();
  await app.getByTestId("nav-skills").click();
  await expect(app.getByTestId("library-template-rust")).toBeVisible();
  await app.getByTestId("workspace-demo").click();
  await app.getByRole("tab", { name: "Templates & network" }).click();
  await expect(app.getByTestId("template-rust")).toBeVisible();
});

test("network tab reports blocked egress", async ({ app }) => {
  await open(app);
  await setApiKey(app);
  await app.locator('[data-testid^="conversation-"]').first().click();
  await app.getByLabel("Message").fill("curl something");
  await app.getByTestId("send").click();
  await expect(app.getByTestId("msg-turn")).toBeVisible();
  await app.getByRole("tab", { name: "Network" }).click();
  await expect(app.getByTestId("denied-summary")).toContainText("blocked");
  await expect(app.locator('[data-testid="egress-row"][data-verdict="deny"]')).toContainText("example.com:443");
});
