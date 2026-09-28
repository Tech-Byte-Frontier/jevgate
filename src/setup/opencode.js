// jevgate:managed: written by `jevgate init --agent opencode`; run it again to update
// this file, or add --remove to take it out.
//
// JevGate for OpenCode 1.x. OpenCode has no command hooks: it loads this module and calls
// the functions below. Each relays an OpenCode event to `jevgate hook --agent opencode`
// and puts the reply where OpenCode carries it: context after an edit is appended to the
// tool's output, the reason a turn's end was blocked is sent as the next prompt, and what
// the person should know is shown as a toast. JevGate's checks, gate and wording are the
// same as in every other agent. Nothing here may throw into OpenCode: every path catches,
// and a missing, failing or slow jevgate is shown to the person, never raised.
import { spawn } from "node:child_process";

/** OpenCode's tools that write files; the hook reads their `filePath` or `patchText`. */
const EDIT_TOOLS = new Set(["edit", "write", "patch", "multiedit", "apply_patch"]);

/**
 * The time each event may take, a little above the hook's own budget (10 s at a turn's
 * start, 30 s after an edit, 50 s at the end of a turn), so the hook answers first.
 */
const TIMEOUT_MS = { "chat.message": 15_000, "tool.execute.after": 35_000, "session.idle": 55_000 };

/** Enough of a reply to recognize what answered instead of JevGate. */
const QUOTED_CHARS = 120;

/** A reply saying why nothing was checked. */
const unchecked = (why) => ({ systemMessage: `JevGate could not check: ${why}. Nothing was blocked.` });

/** Why `jevgate` did not start. */
const notStarted = (error) =>
  unchecked(
    error?.code === "ENOENT"
      ? "jevgate is not on the PATH OpenCode runs with (install JevGate 0.27 or later)"
      : `jevgate did not start (${error?.message ?? error})`,
  );

/** The reply of a `jevgate hook` that exited with `code`: its JSON, or why there is none. */
const replyOf = (code, stdout, stderr) => {
  if (code !== 0) {
    // `jevgate hook` always exits 0: anything else is an older jevgate or a crash.
    const first = stderr.trim().split("\n")[0];
    return unchecked(`jevgate hook exited ${code}${first ? ` (${first})` : ""}; JevGate 0.27 or later is needed`);
  }
  const text = stdout.trim();
  try {
    return text === "" ? {} : JSON.parse(text);
  } catch {
    return unchecked(`jevgate hook answered something other than JSON (${text.slice(0, QUOTED_CHARS)})`);
  }
};

/** Run `jevgate hook` on one event, for at most its time: the reply. */
const runHook = (event, timeouts) =>
  new Promise((resolve) => {
    let child;
    try {
      child = spawn("jevgate", ["hook", "--agent", "opencode"], {
        cwd: event.directory,
        stdio: ["pipe", "pipe", "pipe"],
        windowsHide: true,
      });
    } catch (error) {
      resolve(notStarted(error));
      return;
    }
    const limit = timeouts[event.hook_event_name] ?? timeouts["chat.message"];
    const timer = setTimeout(() => {
      try {
        child.kill();
      } catch {}
      resolve(unchecked(`jevgate hook did not answer within ${Math.round(limit / 1000)} s`));
    }, limit);
    const finish = (reply) => {
      clearTimeout(timer);
      resolve(reply);
    };
    const out = [];
    const err = [];
    // A child that exits before reading stdin makes the write fail later; unheard, it kills OpenCode.
    for (const stream of [child.stdin, child.stdout, child.stderr]) stream.on("error", () => {});
    child.stdout.on("data", (chunk) => out.push(chunk));
    child.stderr.on("data", (chunk) => err.push(chunk));
    child.on("error", (error) => finish(notStarted(error)));
    child.on("close", (code) =>
      finish(replyOf(code, Buffer.concat(out).toString("utf8"), Buffer.concat(err).toString("utf8"))),
    );
    try {
      child.stdin.end(JSON.stringify(event));
    } catch {}
  });

/** The text a prompt's parts carry. */
const textOf = (parts) =>
  (Array.isArray(parts) ? parts : [])
    .filter((part) => part && part.type === "text" && typeof part.text === "string")
    .map((part) => part.text)
    .join("\n");

/** The context a reply gives the agent. */
const contextOf = (reply) => {
  const context = reply?.hookSpecificOutput?.additionalContext;
  return typeof context === "string" && context !== "" ? context : undefined;
};

export const JevGate = async ({ client, directory }) => {
  /** Per session: the block reason sent as a prompt, whether the turn continues one, and whether its end was checked. */
  const sessions = new Map();
  const session = (id) => {
    if (!sessions.has(id)) sessions.set(id, { resubmitted: undefined, continuing: false, stopped: false });
    return sessions.get(id);
  };
  /** Show the person a message; an unhandled rejection is as fatal to OpenCode as a throw. */
  const tell = (message) => {
    if (typeof message !== "string" || message === "") return;
    try {
      const toast = client?.tui?.showToast?.({ body: { message, variant: "warning" } });
      Promise.resolve(toast).catch(() => {});
      const logged = client?.app?.log?.({ body: { service: "jevgate", level: "warn", message } });
      Promise.resolve(logged).catch(() => {});
    } catch {}
  };
  const hook = (event) => runHook({ ...event, directory }, JevGate.timeouts);

  return {
    "chat.message": async (input, output) => {
      try {
        const sessionID = input?.sessionID;
        if (!sessionID) return;
        const prompt = textOf(output?.parts);
        const state = session(sessionID);
        state.continuing = state.resubmitted !== undefined && prompt === state.resubmitted;
        state.resubmitted = undefined;
        state.stopped = false;
        const reply = await hook({ hook_event_name: "chat.message", sessionID, prompt });
        tell(reply?.systemMessage);
        const context = contextOf(reply);
        const messageID = output?.message?.id;
        if (context && messageID && Array.isArray(output.parts)) {
          output.parts.push({
            id: `prt_jevgate_${Date.now().toString(36)}`,
            sessionID,
            messageID,
            type: "text",
            text: context,
            synthetic: true,
          });
        }
      } catch {}
    },

    "tool.execute.after": async (input, output) => {
      try {
        if (!EDIT_TOOLS.has(input?.tool) || !input?.sessionID) return;
        const reply = await hook({
          hook_event_name: "tool.execute.after",
          sessionID: input.sessionID,
          tool: input.tool,
          args: input.args ?? {},
        });
        tell(reply?.systemMessage);
        const context = contextOf(reply);
        if (context && output) output.output = `${output.output ?? ""}\n\n${context}`;
      } catch {}
    },

    event: async ({ event }) => {
      try {
        const idle =
          event?.type === "session.idle" ||
          (event?.type === "session.status" && event?.properties?.status?.type === "idle");
        const sessionID = event?.properties?.sessionID;
        const state = sessions.get(sessionID);
        // One check per prompt: OpenCode can report the same idle twice.
        if (!idle || !state || state.stopped) return;
        state.stopped = true;
        const reply = await hook({ hook_event_name: "session.idle", sessionID, continued: state.continuing });
        tell(reply?.systemMessage);
        if (reply?.decision === "block" && typeof reply.reason === "string") {
          state.resubmitted = reply.reason;
          await client.session.promptAsync({
            path: { id: sessionID },
            body: { parts: [{ type: "text", text: reply.reason }] },
          });
        }
      } catch {}
    },
  };
};

/** How long each event may take; tests shorten them. */
JevGate.timeouts = { ...TIMEOUT_MS };
