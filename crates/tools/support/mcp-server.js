// nucleus MCP server, run by the Claude CLI inside the sandbox over stdio.
//
// Exposes the approved self-built tools (listed in tools.json, written by the harness) and the
// proposal tools the agent uses to suggest new skills, tools and templates. Proposals are only
// written to the outbox; the harness turns them into proposals the user must approve.
'use strict';
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const { spawn } = require('child_process');

const SUPPORT = process.env.NUCLEUS_SUPPORT_DIR || '/nucleus/support';
const OUTBOX = process.env.NUCLEUS_OUTBOX_DIR || '/nucleus/outbox';
const WORKSPACE = process.env.NUCLEUS_WORKSPACE || '/workspace';
const MAX_OUTPUT = 100 * 1024;

const BUILTIN = [
  {
    name: 'propose_skill',
    description:
      'Propose creating or updating a reusable skill after finishing a task. `content` is a full SKILL.md: ' +
      "frontmatter with name (lowercase-dashed), description and when_to_use, then markdown instructions. " +
      'Only propose lessons that generalise beyond this one task. The user reviews every proposal.',
    inputSchema: {
      type: 'object',
      properties: { content: { type: 'string' }, rationale: { type: 'string' } },
      required: ['content', 'rationale'],
    },
  },
  {
    name: 'propose_tool',
    description:
      'Propose promoting a tool you built to the shared tool registry. First create the directory ' +
      `${OUTBOX}/tools/<name>/ with tool.toml (name, description, run, test, [input_schema]), the script, and ` +
      'a test command that exits 0 on success. The harness runs the test in the sandbox, then the user reviews it.',
    inputSchema: {
      type: 'object',
      properties: { name: { type: 'string' }, rationale: { type: 'string' } },
      required: ['name', 'rationale'],
    },
  },
  {
    name: 'propose_template',
    description:
      'Propose a dependency template: a prebuilt dependency directory mounted into future sandboxes. ' +
      '`manifest` is template.toml content (name, mount = { mode = "readonly" | "overlay" | "worktree", path }, env, ' +
      'path_env, [build] lockfiles, command, network). The user reviews and builds it.',
    inputSchema: {
      type: 'object',
      properties: { manifest: { type: 'string' }, rationale: { type: 'string' } },
      required: ['manifest', 'rationale'],
    },
  },
];

function libraryTools() {
  try {
    return JSON.parse(fs.readFileSync(path.join(SUPPORT, 'tools.json'), 'utf8'));
  } catch {
    return [];
  }
}

function writeProposal(kind, payload) {
  const dir = path.join(OUTBOX, 'proposals');
  fs.mkdirSync(dir, { recursive: true });
  const id = crypto.randomUUID();
  const tmp = path.join(dir, `.${id}.tmp`);
  fs.writeFileSync(tmp, JSON.stringify({ kind, ...payload }));
  fs.renameSync(tmp, path.join(dir, `${id}.json`));
  return id;
}

function text(t, isError = false) {
  return { content: [{ type: 'text', text: t }], isError };
}

function runTool(tool, args) {
  return new Promise((resolve) => {
    const input = JSON.stringify(args || {});
    const child = spawn('sh', ['-c', tool.run], {
      cwd: tool.dir,
      env: { ...process.env, NUCLEUS_TOOL_INPUT: input, NUCLEUS_WORKSPACE: WORKSPACE },
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    let out = '';
    let err = '';
    const cap = (s, d) => (s.length < MAX_OUTPUT ? s + d : s);
    child.stdout.on('data', (d) => (out = cap(out, d)));
    child.stderr.on('data', (d) => (err = cap(err, d)));
    const timer = setTimeout(() => child.kill('SIGKILL'), (tool.timeout_secs || 120) * 1000);
    child.on('error', (e) => {
      clearTimeout(timer);
      resolve(text(`failed to start tool: ${e.message}`, true));
    });
    child.on('close', (code, signal) => {
      clearTimeout(timer);
      const body = out + (err ? `\n[stderr]\n${err}` : '');
      if (signal) resolve(text(`tool killed by ${signal} (timeout ${tool.timeout_secs}s)\n${body}`, true));
      else resolve(text(body || '(no output)', code !== 0));
    });
    child.stdin.end(input);
  });
}

async function callTool(name, args) {
  args = args || {};
  switch (name) {
    case 'propose_skill':
      return text(`Skill proposal ${writeProposal('skill', args)} recorded. The user will review it.`);
    case 'propose_template':
      return text(`Template proposal ${writeProposal('template', args)} recorded. The user will review it.`);
    case 'propose_tool': {
      if (!/^[a-z0-9][a-z0-9_-]*$/.test(args.name || '')) return text('invalid tool name', true);
      const dir = path.join(OUTBOX, 'tools', args.name);
      if (!fs.existsSync(path.join(dir, 'tool.toml'))) return text(`missing ${dir}/tool.toml`, true);
      return text(`Tool proposal ${writeProposal('tool', args)} recorded. The harness will run its test, then the user reviews it.`);
    }
    default: {
      const tool = libraryTools().find((t) => t.name === name);
      if (!tool) return text(`unknown tool ${name}`, true);
      return runTool(tool, args);
    }
  }
}

async function handle(msg) {
  const { id, method, params } = msg;
  switch (method) {
    case 'initialize':
      return {
        protocolVersion: (params && params.protocolVersion) || '2025-06-18',
        capabilities: { tools: { listChanged: false } },
        serverInfo: { name: 'nucleus', version: '0.1.0' },
      };
    case 'ping':
      return {};
    case 'tools/list':
      return {
        tools: [
          ...BUILTIN,
          ...libraryTools().map((t) => ({ name: t.name, description: t.description, inputSchema: t.input_schema })),
        ],
      };
    case 'tools/call':
      return callTool(params.name, params.arguments);
    default:
      if (id === undefined) return undefined; // notification
      throw { code: -32601, message: `method not found: ${method}` };
  }
}

let buffer = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', (chunk) => {
  buffer += chunk;
  let nl;
  while ((nl = buffer.indexOf('\n')) >= 0) {
    const line = buffer.slice(0, nl).trim();
    buffer = buffer.slice(nl + 1);
    if (!line) continue;
    let msg;
    try {
      msg = JSON.parse(line);
    } catch {
      process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: null, error: { code: -32700, message: 'parse error' } }) + '\n');
      continue;
    }
    Promise.resolve()
      .then(() => handle(msg))
      .then(
        (result) => {
          if (msg.id !== undefined && result !== undefined)
            process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: msg.id, result }) + '\n');
        },
        (e) => {
          if (msg.id !== undefined)
            process.stdout.write(
              JSON.stringify({ jsonrpc: '2.0', id: msg.id, error: { code: e.code || -32603, message: e.message || String(e) } }) + '\n',
            );
        },
      );
  }
});
