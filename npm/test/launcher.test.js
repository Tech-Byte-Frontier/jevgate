"use strict";
// The npm launcher without the network: platform mapping, SHA256SUMS, the cache, a
// full install from a local fixture archive, and each command's exit contract.
const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const test = require("node:test");
const launcher = require("../lib/launcher.js");

const VERSION = "9.9.9";
/** v0.25.0's SHA256SUMS, as the release workflow writes it: Windows's line has `*`. */
const RELEASE_SUMS = `2aa791d8928db758683939ea27ea3872e8d826eae55d89d6528b64d25d0e6c8e  jevgate-0.25.0-aarch64-apple-darwin.tar.gz
539251dc97ee603ab2c3ef28ca73dd8aea148eed3836b7d0fa23087c7fb151ac  jevgate-0.25.0-aarch64-unknown-linux-musl.tar.gz
a4085f0ac89b87b7a365a8867c3aca76ec264a3baab6f1dcdcee6e67def6dc40  jevgate-0.25.0-x86_64-apple-darwin.tar.gz
9569467ebff6ba439abcce222cddbd857275596a25678b7cd5c36a4af6516583 *jevgate-0.25.0-x86_64-pc-windows-msvc.zip
57638534fa5500101bb604585b5740c9212d91659a2a61e918fe706ca0fc4256  jevgate-0.25.0-x86_64-unknown-linux-musl.tar.gz
`;
const HOST = launcher.target(process.platform, process.arch);

function scratch(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "jevgate-npm-"));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  return directory;
}

/** Streams that remember what was written. */
function streams() {
  const out = { stdout: "", stderr: "" };
  return {
    out,
    stdout: { write: (text) => (out.stdout += text) },
    stderr: { write: (text) => (out.stderr += text) },
  };
}

/**
 * A release archive of `version` for this machine's build, holding a program that
 * records its arguments in `record` and exits 3; and a `fetchBytes` serving it and its
 * SHA256SUMS from memory, counting requests.
 */
function release(directory, record, version = VERSION) {
  const name = launcher.archiveName(version, HOST);
  const folder = path.join(directory, `jevgate-${version}-${HOST}`);
  fs.mkdirSync(folder);
  const program = path.join(folder, process.platform === "win32" ? "jevgate.exe" : "jevgate");
  fs.writeFileSync(program, `#!/bin/sh\necho "$@" > "${record}"\nexit 3\n`, { mode: 0o755 });
  const tar = process.platform === "win32" ? path.join(process.env.SystemRoot, "System32", "tar.exe") : "tar";
  const flags = name.endsWith(".zip") ? ["-a", "-cf"] : ["-czf"];
  const packed = spawnSync(tar, [...flags, name, path.basename(folder)], { cwd: directory });
  assert.equal(packed.status, 0, String(packed.stderr));
  const archive = fs.readFileSync(path.join(directory, name));
  const sums = `${crypto.createHash("sha256").update(archive).digest("hex")}  ${name}\n`;
  const requests = [];
  const fetchBytes = async (url) => {
    requests.push(url);
    if (url.endsWith("/SHA256SUMS")) return Buffer.from(sums);
    if (url.endsWith(`/${name}`)) return archive;
    throw new Error(`unexpected ${url}`);
  };
  return { name, archive, sums, requests, fetchBytes };
}

test("each supported platform maps to its release build, others to none", () => {
  assert.equal(launcher.target("darwin", "arm64"), "aarch64-apple-darwin");
  assert.equal(launcher.target("darwin", "x64"), "x86_64-apple-darwin");
  assert.equal(launcher.target("linux", "x64"), "x86_64-unknown-linux-musl");
  assert.equal(launcher.target("linux", "arm64"), "aarch64-unknown-linux-musl");
  assert.equal(launcher.target("win32", "x64"), "x86_64-pc-windows-msvc");
  assert.equal(launcher.target("win32", "arm64"), "x86_64-pc-windows-msvc");
  for (const [platform, arch] of [["linux", "ia32"], ["linux", "arm"], ["freebsd", "x64"], ["aix", "ppc64"]]) {
    assert.equal(launcher.target(platform, arch), undefined, `${platform} ${arch}`);
  }
});

test("archive names follow the release workflow's", () => {
  assert.equal(launcher.archiveName("0.25.0", "aarch64-apple-darwin"), "jevgate-0.25.0-aarch64-apple-darwin.tar.gz");
  assert.equal(launcher.archiveName("0.25.0", "x86_64-pc-windows-msvc"), "jevgate-0.25.0-x86_64-pc-windows-msvc.zip");
});

test("SHA256SUMS lines are read in both forms, and nothing else is", () => {
  assert.equal(
    launcher.checksum(RELEASE_SUMS, "jevgate-0.25.0-x86_64-pc-windows-msvc.zip"),
    "9569467ebff6ba439abcce222cddbd857275596a25678b7cd5c36a4af6516583",
  );
  assert.equal(
    launcher.checksum(RELEASE_SUMS.replaceAll("\n", "\r\n"), "jevgate-0.25.0-aarch64-apple-darwin.tar.gz"),
    "2aa791d8928db758683939ea27ea3872e8d826eae55d89d6528b64d25d0e6c8e",
  );
  assert.equal(launcher.checksum(RELEASE_SUMS, "jevgate-0.25.0-aarch64-apple-darwin"), undefined);
  assert.equal(launcher.checksum("abc  jevgate.tar.gz\n", "jevgate.tar.gz"), undefined);
});

test("binaries are cached where each platform keeps caches", () => {
  const home = path.join("/", "home", "me");
  assert.equal(launcher.cacheDirectory({}, "darwin", home), path.join(home, "Library", "Caches", "jevgate"));
  assert.equal(launcher.cacheDirectory({}, "linux", home), path.join(home, ".cache", "jevgate"));
  assert.equal(launcher.cacheDirectory({ XDG_CACHE_HOME: "/xdg" }, "linux", home), path.join("/xdg", "jevgate"));
  assert.equal(
    launcher.cacheDirectory({ LOCALAPPDATA: "/local" }, "win32", home),
    path.join("/local", "jevgate", "cache"),
  );
});

test("a checked download is unpacked, cached and then reused", { skip: !HOST }, async (t) => {
  const directory = scratch(t);
  const served = release(directory, path.join(directory, "args"));
  const cache = path.join(directory, "cache");
  const install = () => launcher.ensureBinary({ version: VERSION, build: HOST, cache, fetchBytes: served.fetchBytes });
  const binary = await install();
  assert.equal(binary, path.join(cache, `${VERSION}-${HOST}`, process.platform === "win32" ? "jevgate.exe" : "jevgate"));
  assert.equal(served.requests.length, 2, "SHA256SUMS and the archive");
  assert.deepEqual(fs.readdirSync(cache), [`${VERSION}-${HOST}`], "no work directory is left");
  assert.equal(await install(), binary);
  assert.equal(served.requests.length, 2, "a cached binary is not downloaded again");
});

test("a shipped SHA256SUMS spares downloading the release's", { skip: !HOST }, async (t) => {
  const directory = scratch(t);
  const served = release(directory, path.join(directory, "args"));
  const cache = path.join(directory, "cache");
  await launcher.ensureBinary({ version: VERSION, build: HOST, cache, fetchBytes: served.fetchBytes, sums: served.sums });
  assert.deepEqual(
    served.requests.map((url) => path.posix.basename(url)),
    [served.name],
  );
});

test("an archive that does not match its SHA-256, or is not listed, is refused and nothing is cached", { skip: !HOST }, async (t) => {
  const directory = scratch(t);
  const served = release(directory, path.join(directory, "args"));
  const cache = path.join(directory, "cache");
  const tampered = served.sums.replace(/^./, (c) => (c === "0" ? "1" : "0"));
  await assert.rejects(
    launcher.ensureBinary({ version: VERSION, build: HOST, cache, fetchBytes: served.fetchBytes, sums: tampered }),
    /does not match its SHA-256/,
  );
  await assert.rejects(
    launcher.ensureBinary({ version: VERSION, build: HOST, cache, fetchBytes: served.fetchBytes, sums: RELEASE_SUMS }),
    /lists no jevgate-9\.9\.9/,
  );
  assert.equal(fs.existsSync(path.join(cache, `${VERSION}-${HOST}`)), false);
});

test("the launcher runs the binary with the same arguments and exit code", { skip: !HOST || process.platform === "win32" }, async (t) => {
  const directory = scratch(t);
  const record = path.join(directory, "args");
  const served = release(directory, record, require("../package.json").version);
  const home = path.join(directory, "home");
  const env = { XDG_CACHE_HOME: path.join(directory, "xdg") };
  const captured = streams();
  // `sums: null`: no shipped copy, even when one was downloaded into the package to publish it.
  const run = () => launcher.main(["check", "--base", "HEAD"], { env, home, streams: captured, fetchBytes: served.fetchBytes, sums: null });
  assert.equal(await run(), 3);
  assert.equal(fs.readFileSync(record, "utf8").trim(), "check --base HEAD");
  assert.match(captured.out.stderr, /downloading JevGate/);
  assert.equal(captured.out.stdout, "", "the launcher never writes to stdout");
  assert.equal(await run(), 3);
  assert.equal(served.requests.length, 2, "the second run starts the cached binary");
});

test("a failed install keeps each command's exit contract", () => {
  const hook = streams();
  assert.equal(launcher.failure(["hook"], "could not install JevGate 1.0.0: offline", hook), 0);
  const reply = JSON.parse(hook.out.stdout);
  assert.match(reply.systemMessage, /could not install JevGate 1\.0\.0: offline\. Nothing was checked or blocked\.$/);
  assert.match(hook.out.stderr, /offline/);
  const check = streams();
  assert.equal(launcher.failure(["check"], "could not install", check), 2);
  assert.equal(check.out.stdout, "");
});

test("an unsupported machine is told, in the hook's own reply", async () => {
  const captured = streams();
  const code = await launcher.main(["hook"], { platform: "aix", arch: "ppc64", streams: captured });
  assert.equal(code, 0);
  assert.match(JSON.parse(captured.out.stdout).systemMessage, /no release build for aix ppc64/);
});

test("the package runs nothing at install and exposes one bin", () => {
  const manifest = require("../package.json");
  assert.equal(manifest.scripts, undefined);
  assert.equal(manifest.dependencies, undefined);
  assert.deepEqual(manifest.bin, { jevgate: "bin/jevgate.js" });
});
