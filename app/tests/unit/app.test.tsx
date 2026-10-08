import { fireEvent, waitFor, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { describe, expect, test } from "vitest";
import { renderApp, withKey } from "./helpers";

describe("startup", () => {
  test("shows the engine error and recovers on retry", async () => {
    const r = renderApp({ initError: "no Podman or Docker socket found" });
    expect(await r.findByTestId("init-error")).toHaveTextContent("no Podman or Docker socket found");
    r.backend.setInitError(null);
    fireEvent.click(r.getByTestId("retry-init"));
    expect(await r.findByTestId("workspace-view")).toBeInTheDocument();
  });

  test("backend failures during init show the error screen", async () => {
    const r = renderApp();
    await r.findByTestId("workspace-view");
    r.backend.failNext("init", "IPC exploded");
    await r.app.actions.init();
    expect(await r.findByTestId("init-error")).toHaveTextContent("IPC exploded");
  });

  test("empty state without workspaces", async () => {
    const r = renderApp({ seed: false });
    expect(await r.findByText("No workspace yet")).toBeInTheDocument();
  });
});

describe("workspaces", () => {
  test("add, reject duplicates and invalid paths, remove", async () => {
    const user = userEvent.setup();
    const r = renderApp({ seed: false });
    await r.findByText("No workspace yet");
    await user.click(r.getByTestId("add-workspace"));
    const input = r.getByLabelText("Repository path");
    await user.type(input, "relative");
    await user.click(r.getByRole("button", { name: "Add" }));
    expect(await r.findByTestId("toast-error")).toHaveTextContent("no git repository");
    await user.clear(input);
    await user.type(input, "/src/app");
    await user.click(r.getByRole("button", { name: "Add" }));
    expect(await r.findByTestId("workspace-app")).toBeInTheDocument();
    expect(await r.findByRole("heading", { name: "app" })).toBeInTheDocument();
    await user.click(r.getByTestId("add-workspace"));
    await user.type(r.getByLabelText("Repository path"), "/src/app");
    await user.click(r.getByRole("button", { name: "Add" }));
    await waitFor(() => expect(r.getAllByTestId("toast-error").at(-1)).toHaveTextContent("already a workspace"));
    await user.click(r.getByTestId("remove-workspace"));
    expect(await r.findByText("No workspace yet")).toBeInTheDocument();
  });

  test("template conflicts are shown inline and nothing is saved", async () => {
    const user = userEvent.setup();
    const r = renderApp();
    await r.findByTestId("workspace-view");
    await user.click(r.getByRole("tab", { name: "Templates & network" }));
    await user.click(await r.findByLabelText("Use template python"));
    await user.click(r.getByLabelText("Use template python-alt"));
    await user.click(r.getByTestId("save-workspace"));
    expect(await r.findByTestId("workspace-error")).toHaveTextContent("both set VIRTUAL_ENV");
    expect(r.backend.workspaces[0].templates).toEqual([]);
  });

  test("template order, network allowlist validation and build", async () => {
    const user = userEvent.setup();
    const r = renderApp();
    await r.findByTestId("workspace-view");
    await user.click(r.getByRole("tab", { name: "Templates & network" }));
    await user.click(await r.findByLabelText("Use template node"));
    await user.click(r.getByLabelText("Use template python"));
    await user.click(r.getByLabelText("Move python up"));
    await user.click(r.getByRole("radio", { name: /^Allowlist/ }));
    const hosts = r.getByLabelText("Allowed hosts");
    await user.type(hosts, "pypi.org bad_host");
    expect(r.getByTestId("invalid-hosts")).toHaveTextContent("bad_host");
    expect(r.getByTestId("save-workspace")).toBeDisabled();
    await user.clear(hosts);
    await user.type(hosts, "https://pypi.org/simple");
    await user.click(r.getByTestId("save-workspace"));
    await waitFor(() => expect(r.backend.workspaces[0].templates).toEqual(["python", "node"]));
    expect(r.backend.workspaces[0].network).toEqual({ mode: "allowlist", hosts: ["pypi.org"] });
    expect(await within(r.getByTestId("template-status")).findAllByText("needs build")).toHaveLength(2);
    await user.click(r.getByTestId("build-templates"));
    expect(await within(r.getByTestId("template-status")).findAllByText("built")).toHaveLength(2);
  });

  test("branch tree and compare", async () => {
    const user = userEvent.setup();
    const r = renderApp();
    expect(await r.findAllByTestId("graph-row")).toHaveLength(4);
    await user.click(r.getByTestId("branch-main"));
    await user.click(r.getByTestId("branch-local/feature"));
    await user.click(r.getByTestId("compare-button"));
    expect(await r.findByTestId("diff-file-docs/feature.md")).toBeInTheDocument();
    await user.click(r.getByTestId("branch-main"));
    await user.click(r.getByTestId("branch-origin/main"));
    await user.click(r.getByTestId("compare-button"));
    expect(await r.findByText("The branches have the same content")).toBeInTheDocument();
  });
});

describe("conversations", () => {
  test("start a conversation, chat streams, changes appear", async () => {
    const user = userEvent.setup();
    const r = renderApp();
    await withKey(r.backend);
    await r.findByTestId("new-conversation");
    await user.type(r.getByLabelText("Conversation title"), "Write docs");
    await user.selectOptions(r.getByLabelText("Base branch"), "local/feature");
    await user.click(r.getByTestId("start-conversation"));
    expect(await r.findByTestId("conversation-view")).toBeInTheDocument();
    const conv = r.backend.conversations.at(-1)!;
    expect(conv.base_branch).toBe("local/feature");
    expect(r.getByTestId("conversation-title")).toHaveTextContent("Write docs");

    const send = r.getByTestId("send");
    expect(send).toBeDisabled();
    await user.type(r.getByLabelText("Message"), "Document everything");
    await user.click(send);
    expect(await r.findByText(/Done\. I updated notes\.md/)).toBeInTheDocument();
    expect(await r.findByTestId("msg-turn")).toHaveTextContent("turn complete");
    expect(r.getByTestId("msg-user")).toHaveTextContent("Document everything");
    expect(r.getAllByTestId("msg-tool")).toHaveLength(1);
    await user.click(within(r.getByTestId("msg-tool")).getByRole("button"));
    expect(r.getByTestId("tool-result")).toHaveTextContent("README.md");

    await user.click(r.getByRole("tab", { name: "Changes" }));
    expect(await r.findByTestId("diff-file-notes.md")).toBeInTheDocument();
    expect(r.getByTestId("unmerged-commits").children).toHaveLength(1);
  });

  test("ctrl+enter sends, missing credentials become a chat error", async () => {
    const user = userEvent.setup();
    const r = renderApp();
    const conv = r.backend.conversations[0];
    fireEvent.click(await r.findByTestId(`conversation-${conv.id}`));
    expect(await r.findByText(/I added a unit test/)).toBeInTheDocument();
    await user.type(r.getByLabelText("Message"), "hello{Control>}{Enter}{/Control}");
    expect(await r.findByTestId("msg-error")).toHaveTextContent("ANTHROPIC_API_KEY");
    expect(r.getByLabelText("Message")).toHaveValue("");
  });

  test("a failing turn shows an error and the error status", async () => {
    const user = userEvent.setup();
    const r = renderApp();
    await withKey(r.backend);
    const conv = r.backend.conversations[0];
    fireEvent.click(await r.findByTestId(`conversation-${conv.id}`));
    await user.type(await r.findByLabelText("Message"), "please fail");
    await user.click(r.getByTestId("send"));
    expect(await r.findByText(/exited with code 1/)).toBeInTheDocument();
    await waitFor(() => expect(r.getAllByTestId("status-dot")[0]).toHaveAttribute("data-status", "error"));
  });

  test("stop cancels a running turn", async () => {
    const user = userEvent.setup();
    const r = renderApp({ delayMs: 10 });
    await withKey(r.backend);
    const conv = r.backend.conversations[0];
    fireEvent.click(await r.findByTestId(`conversation-${conv.id}`));
    await user.type(await r.findByLabelText("Message"), "slow task");
    await user.click(r.getByTestId("send"));
    await user.click(await r.findByTestId("stop"));
    expect(await r.findByTestId("send")).toBeInTheDocument();
    expect(r.backend.calls.some(([m]) => m === "cancel")).toBe(true);
    await waitFor(() => expect(r.backend.conversations[0].status).toBe("error"));
  });

  test("rename", async () => {
    const user = userEvent.setup();
    const r = renderApp();
    const conv = r.backend.conversations[0];
    fireEvent.click(await r.findByTestId(`conversation-${conv.id}`));
    await user.click(await r.findByTestId("conversation-title"));
    const input = r.getByLabelText("Conversation title");
    await user.clear(input);
    await user.type(input, "Renamed{Enter}");
    expect(await r.findByTestId("conversation-title")).toHaveTextContent("Renamed");
    expect(r.getByTestId(`conversation-${conv.id}`)).toHaveTextContent("Renamed");
  });

  test("merge from the changes tab", async () => {
    const user = userEvent.setup();
    const r = renderApp();
    const conv = r.backend.conversations[0];
    fireEvent.click(await r.findByTestId(`conversation-${conv.id}`));
    await user.click(await r.findByRole("tab", { name: "Changes" }));
    expect(await r.findByLabelText("Merge target")).toHaveValue("main");
    await user.click(r.getByTestId("merge"));
    expect(await r.findByTestId("toast-success")).toHaveTextContent("Merged into main");
    await waitFor(() => expect(r.queryByTestId("unmerged-commits")).not.toBeInTheDocument());
  });
});

describe("delete dialog", () => {
  const open = async () => {
    const user = userEvent.setup();
    const r = renderApp();
    const conv = r.backend.conversations[0];
    fireEvent.click(await r.findByTestId(`conversation-${conv.id}`));
    await user.click(await r.findByTestId("delete-conversation"));
    await user.click(r.getByTestId("confirm-delete"));
    expect(await r.findByTestId("unmerged-warning")).toHaveTextContent("1 commit is only on this agent branch");
    return { r, user, conv };
  };

  test("discard", async () => {
    const { r, user, conv } = await open();
    await user.click(r.getByTestId("discard"));
    await waitFor(() => expect(r.queryByTestId("delete-dialog")).not.toBeInTheDocument());
    expect(r.queryByTestId(`conversation-${conv.id}`)).not.toBeInTheDocument();
    expect(await r.findByTestId("workspace-view")).toBeInTheDocument();
  });

  test("keep a copy, refusing names in agent/", async () => {
    const { r, user } = await open();
    const name = r.getByLabelText("Copy branch name");
    await user.clear(name);
    await user.type(name, "agent/nope");
    await user.click(r.getByTestId("keep-copy"));
    expect(await r.findByTestId("toast-error")).toHaveTextContent("outside the agent/ namespace");
    expect(r.getByTestId("delete-dialog")).toBeInTheDocument();
    await user.clear(name);
    await user.type(name, "local/kept");
    await user.click(r.getByTestId("keep-copy"));
    await waitFor(() => expect(r.queryByTestId("delete-dialog")).not.toBeInTheDocument());
    expect((await r.backend.branches(r.backend.workspaces[0].id)).map((b) => b.name)).toContain("local/kept");
  });

  test("merge conflicts keep the conversation", async () => {
    const { r, user, conv } = await open();
    r.backend.commitOnBranch(conv.workspace_id, "main", "src/lib.rs", "other\n");
    await user.click(r.getByTestId("merge-and-delete"));
    expect(await r.findByTestId("merge-conflicts")).toHaveTextContent("src/lib.rs");
    expect(r.backend.conversations).toHaveLength(1);
    await user.click(r.getByRole("button", { name: "Close" }));
    expect(r.queryByTestId("delete-dialog")).not.toBeInTheDocument();
  });

  test("clean conversations delete without confirmation", async () => {
    const user = userEvent.setup();
    const r = renderApp();
    await r.findByTestId("new-conversation");
    await user.click(r.getByTestId("start-conversation"));
    await r.findByTestId("conversation-view");
    await user.click(r.getByTestId("delete-conversation"));
    await user.click(r.getByTestId("confirm-delete"));
    await waitFor(() => expect(r.backend.conversations).toHaveLength(1));
    expect(r.queryByTestId("unmerged-warning")).not.toBeInTheDocument();
  });
});

describe("proposals and skills", () => {
  test("review, approve and reject; badge follows", async () => {
    const user = userEvent.setup();
    const r = renderApp();
    expect(await r.findByTestId("proposal-count")).toHaveTextContent("2");
    await user.click(r.getByTestId("nav-proposals"));
    expect(await r.findByTestId("proposal-detail")).toBeInTheDocument();
    expect(await r.findByTestId("diff-view")).toBeInTheDocument();
    const first = r.backend.calls.filter(([m]) => m === "proposal").length;
    expect(first).toBeGreaterThan(0);
    await user.click(r.getByTestId("approve"));
    await waitFor(() => expect(r.getByTestId("proposal-count")).toHaveTextContent("1"));
    await user.click(r.getByTestId("reject"));
    expect(await r.findByText("No pending proposals")).toBeInTheDocument();
    expect(r.queryByTestId("proposal-count")).not.toBeInTheDocument();
  });

  test("proposal events from a turn show up live", async () => {
    const user = userEvent.setup();
    const r = renderApp();
    await withKey(r.backend);
    const conv = r.backend.conversations[0];
    fireEvent.click(await r.findByTestId(`conversation-${conv.id}`));
    await user.type(await r.findByLabelText("Message"), "learn this");
    await user.click(r.getByTestId("send"));
    await waitFor(() => expect(r.getByTestId("proposal-count")).toHaveTextContent("3"));
    expect(r.getAllByTestId("toast-info").some((t) => t.textContent?.includes("notes-style"))).toBe(true);
    expect(r.getAllByTestId("toast-error").some((t) => t.textContent?.includes("rejected automatically"))).toBe(true);
  });

  test("skills are flagged and approvals can be reverted", async () => {
    const user = userEvent.setup();
    const r = renderApp();
    await r.findByTestId("workspace-view");
    await user.click(r.getByTestId("nav-skills"));
    expect(await r.findByTestId("skill-rust-style")).toHaveAttribute("data-flagged", "true");
    await user.click(r.getByTestId("nav-proposals"));
    await r.findByTestId("proposal-detail");
    // Approve the skill proposal specifically.
    const skill = (await r.backend.proposals()).find((p) => p.kind === "skills")!;
    await user.click(r.getByTestId(`proposal-${skill.id}`));
    await user.click(r.getByTestId("approve"));
    await waitFor(() => expect(r.getByTestId("proposal-count")).toHaveTextContent("1"));
    await user.click(r.getByTestId("nav-skills"));
    expect(await r.findByTestId("skill-cargo-tests")).toHaveAttribute("data-flagged", "false");
    const revert = (await r.findAllByRole("button", { name: "Revert" }))[0];
    await user.click(revert);
    await waitFor(() => expect(r.queryByTestId("skill-cargo-tests")).not.toBeInTheDocument());
  });
});

describe("settings", () => {
  test("secrets are write-only and can be removed", async () => {
    const user = userEvent.setup();
    const r = renderApp();
    await r.findByTestId("workspace-view");
    await user.click(r.getByTestId("nav-settings"));
    await user.type(r.getByLabelText("ANTHROPIC_API_KEY"), "sk-secret");
    await user.click(r.getByTestId("save-settings"));
    expect(await r.findByTestId("toast-success")).toHaveTextContent("Settings saved");
    // The stored secret is never sent back to the UI.
    expect(await r.findByPlaceholderText("stored")).toHaveValue("");
    expect((await r.backend.state()).settings.provider_env.ANTHROPIC_API_KEY).not.toBe("sk-secret");
    // Saving other changes keeps it.
    await user.type(r.getByLabelText("Model"), "opus");
    await user.click(r.getByTestId("save-settings"));
    await waitFor(() => expect(r.backend.settings.model).toBe("opus"));
    await r.backend.sendMessage(r.backend.conversations[0].id, "works with stored key");
    // Remove it.
    await user.click(await r.findByTestId("clear-ANTHROPIC_API_KEY"));
    await user.click(r.getByTestId("save-settings"));
    await waitFor(async () => expect((await r.backend.state()).settings.provider_env).toEqual({}));
  });

  test("empty image cannot be saved; maintenance actions report back", async () => {
    const user = userEvent.setup();
    const r = renderApp();
    await r.findByTestId("workspace-view");
    await user.click(r.getByTestId("nav-settings"));
    await user.clear(r.getByLabelText("Agent image"));
    expect(r.getByTestId("save-settings")).toBeDisabled();
    await user.click(r.getByTestId("cleanup"));
    expect(await r.findByText("Nothing to clean up")).toBeInTheDocument();
    await user.click(r.getByTestId("build-image"));
    expect(await r.findByText("Agent image built")).toBeInTheDocument();
  });
});

describe("logs", () => {
  test("shows harness logs, filters by level and text", async () => {
    const user = userEvent.setup();
    const r = renderApp();
    await r.findByTestId("workspace-view");
    r.backend.log("error", "orphan cleanup failed: boom");
    r.backend.log("info", "conversation created conversation=abc");
    await user.click(r.getByTestId("nav-logs"));
    await waitFor(() => expect(r.getAllByTestId("log-row").length).toBeGreaterThanOrEqual(2));
    expect(r.queryAllByTestId("log-row").every((row) => row.dataset.level !== "debug")).toBe(true);
    await user.type(r.getByLabelText("Filter logs"), "orphan");
    await waitFor(() => expect(r.getAllByTestId("log-row")).toHaveLength(1));
    await user.clear(r.getByLabelText("Filter logs"));
    await user.selectOptions(r.getByLabelText("Minimum level"), "error");
    await waitFor(() => expect(r.getAllByTestId("log-row")).toHaveLength(1));
    await user.selectOptions(r.getByLabelText("Minimum level"), "debug");
    await waitFor(() => expect(r.getAllByTestId("log-row").some((row) => row.dataset.level === "debug")).toBe(true));
  });

  test("a failing log fetch shows an error", async () => {
    const user = userEvent.setup();
    const r = renderApp();
    await r.findByTestId("workspace-view");
    r.backend.failNext("recentLogs", "no logs for you");
    await user.click(r.getByTestId("nav-logs"));
    expect(await r.findByText("no logs for you")).toBeInTheDocument();
    await user.click(r.getByTestId("refresh-logs"));
    await waitFor(() => expect(r.queryByText("no logs for you")).not.toBeInTheDocument());
  });
});

describe("sandbox settings", () => {
  test("limits and permission mode validate, save, and restart applies them", async () => {
    const user = userEvent.setup();
    const r = renderApp();
    await r.findByTestId("workspace-view");
    await user.click(r.getByTestId("nav-settings"));
    await user.selectOptions(await r.findByLabelText("Permission mode"), "acceptEdits");
    expect(r.getByTestId("permission-help")).toHaveTextContent("Edit files freely");
    const mem = r.getByLabelText("Memory limit");
    await user.clear(mem);
    await user.type(mem, "100");
    expect(await r.findByTestId("settings-invalid")).toHaveTextContent("memory limit");
    expect(r.getByTestId("save-settings")).toBeDisabled();
    await user.clear(mem);
    await user.type(mem, "2048");
    await user.type(r.getByLabelText("CPU limit"), "abc");
    expect(await r.findByTestId("settings-invalid")).toHaveTextContent("CPU");
    await user.clear(r.getByLabelText("CPU limit"));
    await user.type(r.getByLabelText("CPU limit"), "1.5");
    await user.clear(r.getByLabelText("Process limit"));
    await waitFor(() => expect(r.queryByTestId("settings-invalid")).not.toBeInTheDocument());
    await user.click(r.getByTestId("save-settings"));
    await waitFor(() => expect(r.backend.settings.limits).toEqual({ memory_mb: 2048, cpus: 1.5, pids: null }));
    expect(r.backend.settings.permission_mode).toBe("acceptEdits");

    const conv = r.backend.conversations[0];
    fireEvent.click(r.getByTestId(`conversation-${conv.id}`));
    await user.click(await r.findByTestId("restart-sandbox"));
    expect(await r.findByText("Sandbox restarted with the current settings")).toBeInTheDocument();
    expect(r.backend.calls.some(([m]) => m === "restartSandbox")).toBe(true);
  });
});
