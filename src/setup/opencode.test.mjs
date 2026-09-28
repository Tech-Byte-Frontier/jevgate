// The OpenCode plugin (opencode.js) against a fake `jevgate` on PATH: what it relays
// to `jevgate hook --agent opencode`, where each reply goes, and failures shown to the
// person instead of thrown into OpenCode. Run with `node --test`.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const PLUGIN = path.join(path.dirname(fileURLToPath(import.meta.url)), "opencode.js");
// The fake jevgate is a script with a shebang, which Windows does not run by name.
const skip = process.platform === "win32";

/**
 * A fake jevgate first on PATH, in a directory removed after the test, that records
 * each event it reads and answers by event name (`replies`), or fails as asked; with
 * `onPath` false, PATH holds only its empty directory. The directory, and the events
 * it read.
 */
function fakeJevgate(t, { replies = {}, exit = 0, stderr = "", stdout, delayMs = 0, onPath = true }) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "jevgate-opencode-"));
  const bin = path.join(directory, "bin");
  const log = path.join(directory, "events.jsonl");
  fs.mkdirSync(bin);
  if (onPath) {
    const answer = stdout ?? null;
    fs.writeFileSync(
      path.join(bin, "jevgate"),
      `#!${process.execPath}
const fs = require("node:fs");
const input = fs.readFileSync(0, "utf8");
fs.appendFileSync(${JSON.stringify(log)}, input + "\\n");
const replies = ${JSON.stringify(replies)};
const reply = ${JSON.stringify(answer)} ?? JSON.stringify(replies[JSON.parse(input).hook_event_name] ?? {});
setTimeout(() => {
  process.stderr.write(${JSON.stringify(stderr)});
  process.stdout.write(reply);
  process.exitCode = ${exit};
}, ${delayMs});
`,
      { mode: 0o755 },
    );
  }
  const path_ = process.env.PATH;
  process.env.PATH = `${bin}${path.delimiter}${onPath ? path_ : ""}`;
  t.after(() => {
    process.env.PATH = path_;
    fs.rmSync(directory, { recursive: true, force: true });
  });
  const events = () =>
    fs.existsSync(log)
      ? fs.readFileSync(log, "utf8").trim().split("\n").map((line) => JSON.parse(line))
      : [];
  return { directory, events };
}

/**
 * The plugin loaded as OpenCode loads it, beside a fake jevgate (`fakeJevgate`'s
 * options), with a client that records the prompts it sends and the toasts it shows.
 */
async function opencode(t, options = {}) {
  const { directory, events } = fakeJevgate(t, options);
  // A copy per test, so each gets a fresh module and its own timeouts.
  const copy = path.join(directory, "jevgate.mjs");
  fs.copyFileSync(PLUGIN, copy);
  const { JevGate } = await import(pathToFileURL(copy).href);
  const prompts = [];
  const toasts = [];
  const client = {
    session: { promptAsync: async (request) => prompts.push(request) },
    tui: { showToast: async ({ body }) => toasts.push(body.message) },
    app: { log: async () => {} },
  };
  const hooks = await JevGate({ client, directory });
  return { JevGate, hooks, prompts, toasts, events, directory };
}

const prompt = (hooks, text, id = "m1") => {
  const output = { message: { id }, parts: [{ type: "text", text }] };
  return hooks["chat.message"]({ sessionID: "s1" }, output).then(() => output);
};
const idle = (hooks, type = "session.idle") =>
  hooks.event({
    event:
      type === "session.idle"
        ? { type, properties: { sessionID: "s1" } }
        : { type, properties: { sessionID: "s1", status: { type: "idle" } } },
  });

test("a prompt starts a turn, and a notice for the agent joins the prompt", { skip }, async (t) => {
  const notice = "JevGate could not check the last turn's changes (HTTP 402), so it was not reviewed; this is not a pass.";
  const { hooks, events, directory } = await opencode(t, {
    replies: { "chat.message": { hookSpecificOutput: { hookEventName: "chat.message", additionalContext: notice } } },
  });
  const output = await prompt(hooks, "Fix the parser");
  assert.deepEqual(events(), [{ hook_event_name: "chat.message", sessionID: "s1", prompt: "Fix the parser", directory }]);
  const added = output.parts.at(-1);
  assert.equal(added.text, notice);
  assert.equal(added.synthetic, true);
  assert.equal(added.messageID, "m1");
});

test("an edit's findings are appended to the tool's output, and other tools are not relayed", { skip }, async (t) => {
  const context = "JevGate reviewed src/a.ts after this edit: 1 finding, 1 fails the quality gate.";
  const { hooks, events } = await opencode(t, {
    replies: { "tool.execute.after": { hookSpecificOutput: { hookEventName: "tool.execute.after", additionalContext: context } } },
  });
  const output = { output: "Edit applied." };
  await hooks["tool.execute.after"]({ tool: "edit", sessionID: "s1", args: { filePath: "src/a.ts" } }, output);
  assert.equal(output.output, `Edit applied.\n\n${context}`);
  await hooks["tool.execute.after"]({ tool: "read", sessionID: "s1", args: { filePath: "src/a.ts" } }, { output: "" });
  const relayed = events();
  assert.equal(relayed.length, 1);
  assert.deepEqual(relayed[0].args, { filePath: "src/a.ts" });
  assert.equal(relayed[0].tool, "edit");
});

test("a blocked stop is sent back as the next prompt, which continues the turn", { skip }, async (t) => {
  const reason = "JevGate blocked the end of this turn (1 of at most 3): 1 finding in code changed this turn fails the quality gate.";
  const { hooks, prompts, events } = await opencode(t, {
    replies: { "session.idle": { decision: "block", reason, systemMessage: "JevGate: 1 finding fails." } },
  });
  await prompt(hooks, "Add the cache");
  await idle(hooks);
  assert.deepEqual(prompts, [{ path: { id: "s1" }, body: { parts: [{ type: "text", text: reason }] } }]);
  await idle(hooks, "session.status");
  assert.equal(prompts.length, 1, "one stop per prompt, however OpenCode reports it");
  await prompt(hooks, reason, "m2");
  await idle(hooks);
  await prompt(hooks, "Thanks", "m3");
  await idle(hooks);
  const stops = events().filter((event) => event.hook_event_name === "session.idle");
  assert.deepEqual(
    stops.map((stop) => stop.continued),
    [false, true, false],
  );
});

test("a stop in a session with no prompt seen is not relayed", { skip }, async (t) => {
  const { hooks, events } = await opencode(t);
  await idle(hooks);
  assert.deepEqual(events(), []);
});

test("an older jevgate, bad output, no jevgate or a slow one is shown, never thrown", { skip }, async (t) => {
  const cases = [
    [{ exit: 2, stderr: "error: unrecognized subcommand 'hook'\n" }, /exited 2 \(error: unrecognized subcommand 'hook'\).*0\.27 or later/],
    [{ stdout: "not json" }, /answered something other than JSON \(not json\)/],
    [{ onPath: false }, /not on the PATH OpenCode runs with/],
    [{ delayMs: 3000 }, /did not answer within 1 s/],
  ];
  for (const [fake, message] of cases) {
    await t.test(String(message), async (t) => {
      const { JevGate, hooks, toasts } = await opencode(t, fake);
      JevGate.timeouts["tool.execute.after"] = 1000;
      const output = { output: "Edit applied." };
      await hooks["tool.execute.after"]({ tool: "write", sessionID: "s1", args: { filePath: "a.ts" } }, output);
      assert.equal(toasts.length, 1);
      assert.match(toasts[0], message);
      assert.match(toasts[0], /Nothing was blocked\.$/);
      assert.equal(output.output, "Edit applied.", "nothing reaches the model");
    });
  }
});
