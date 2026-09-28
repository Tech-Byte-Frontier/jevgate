"use strict";
// Runs JevGate's release binary for this machine. The first run downloads the
// archive of this package's version from the GitHub release, checks it against the
// release's SHA-256 sums, unpacks it with the system `tar` and caches the binary;
// every run then starts the cached binary with the same arguments and exits with its
// code. Nothing runs at install time: the package has no install scripts and no
// dependencies. The launcher's own messages go to stderr, so a command's output
// (the JSON a hook or an MCP client reads) stays the binary's alone.
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const RELEASES = "https://github.com/Tech-Byte-Frontier/jevgate/releases/download";
const INSTALL_PAGE = "https://tech-byte-frontier.github.io/jevgate/install.html";
/** A release archive is about 10 MB (0.30.0); a download slower than this has stalled. */
const DOWNLOAD_TIMEOUT_MS = 5 * 60 * 1000;

/** Release builds by Node's platform and architecture. Windows on Arm runs the x64 build. */
const TARGETS = {
  "darwin arm64": "aarch64-apple-darwin",
  "darwin x64": "x86_64-apple-darwin",
  "linux arm64": "aarch64-unknown-linux-musl",
  "linux x64": "x86_64-unknown-linux-musl",
  "win32 arm64": "x86_64-pc-windows-msvc",
  "win32 x64": "x86_64-pc-windows-msvc",
};

/** The release build for a platform and architecture, or undefined when there is none. */
function target(platform, arch) {
  return TARGETS[`${platform} ${arch}`];
}

const isWindows = (build) => build.includes("windows");

/** The release archive of `version` for `build`. */
function archiveName(version, build) {
  return `jevgate-${version}-${build}${isWindows(build) ? ".zip" : ".tar.gz"}`;
}

function binaryName(build) {
  return isWindows(build) ? "jevgate.exe" : "jevgate";
}

/** The SHA-256 a SHA256SUMS file lists for `name` (`HASH  NAME` or `HASH *NAME`), or undefined. */
function checksum(sums, name) {
  for (const line of sums.split(/\r?\n/)) {
    const match = /^([0-9a-fA-F]{64}) [ *](.+)$/.exec(line.trim());
    if (match && match[2] === name) return match[1].toLowerCase();
  }
  return undefined;
}

/** Where downloaded binaries are kept, by each platform's convention for caches. */
function cacheDirectory(env, platform, home) {
  if (platform === "win32") {
    return path.join(env.LOCALAPPDATA || path.join(home, "AppData", "Local"), "jevgate", "cache");
  }
  if (platform === "darwin") return path.join(home, "Library", "Caches", "jevgate");
  return path.join(env.XDG_CACHE_HOME || path.join(home, ".cache"), "jevgate");
}

/**
 * The cached binary of `version` for `build`, downloaded, checked and unpacked first
 * when missing. `fetchBytes(url)` returns a URL's bytes. `sums` is the release's
 * SHA256SUMS when the package ships a copy, which ties the package to the binaries
 * released with it; without one, the release's own is downloaded.
 */
async function ensureBinary({ version, build, cache, fetchBytes, sums, log = () => {} }) {
  const binary = path.join(cache, `${version}-${build}`, binaryName(build));
  if (fs.existsSync(binary)) return binary;
  log(`downloading JevGate ${version} for ${build}, once; it is kept in ${cache}`);
  const archive = await checkedArchive({ version, build, fetchBytes, sums });
  install(archive, { version, build, cache, binary });
  return binary;
}

/** The release archive of `version` for `build`, once its SHA-256 matches SHA256SUMS. */
async function checkedArchive({ version, build, fetchBytes, sums }) {
  const name = archiveName(version, build);
  const release = `${RELEASES}/v${version}`;
  const listed = sums ?? (await fetchBytes(`${release}/SHA256SUMS`)).toString("utf8");
  const expected = checksum(listed, name);
  if (!expected) throw new Error(`the release's SHA256SUMS lists no ${name}`);
  const archive = await fetchBytes(`${release}/${name}`);
  const actual = crypto.createHash("sha256").update(archive).digest("hex");
  if (actual !== expected) {
    throw new Error(`${name} does not match its SHA-256 in SHA256SUMS (expected ${expected}, got ${actual})`);
  }
  return archive;
}

/**
 * Unpack `archive` in a work directory inside the cache and move its binary to
 * `binary` in one rename, so a launcher never starts half a file.
 */
function install(archive, { version, build, cache, binary }) {
  const name = archiveName(version, build);
  fs.mkdirSync(cache, { recursive: true });
  const work = fs.mkdtempSync(path.join(cache, `.${version}-${build}-`));
  try {
    const saved = path.join(work, name);
    fs.writeFileSync(saved, archive);
    unpack(saved, work);
    const unpacked = path.join(work, `jevgate-${version}-${build}`, binaryName(build));
    if (!fs.existsSync(unpacked)) throw new Error(`${name} holds no ${binaryName(build)}`);
    fs.chmodSync(unpacked, 0o755);
    fs.mkdirSync(path.dirname(binary), { recursive: true });
    try {
      fs.renameSync(unpacked, binary);
    } catch (error) {
      // Another launcher, started at the same time, put the same binary there first.
      if (!fs.existsSync(binary)) throw error;
    }
  } finally {
    fs.rmSync(work, { recursive: true, force: true });
  }
}

/**
 * Unpack an archive with the system tar. Windows 10 and later ship bsdtar, which also
 * reads zip, as System32\tar.exe; Git's GNU tar, often first on PATH there, does not.
 */
function unpack(archive, into) {
  const zip = archive.endsWith(".zip");
  const tar =
    process.platform === "win32" ? path.join(process.env.SystemRoot || "C:\\Windows", "System32", "tar.exe") : "tar";
  const result = spawnSync(tar, [zip ? "-xf" : "-xzf", archive, "-C", into], {
    stdio: ["ignore", "ignore", "pipe"],
    windowsHide: true,
  });
  if (result.error) throw new Error(`tar, which unpacks the download, did not run (${result.error.message})`);
  if (result.status !== 0) {
    throw new Error(`tar could not unpack ${path.basename(archive)} (${String(result.stderr).trim()})`);
  }
}

async function download(url) {
  const response = await fetch(url, { signal: AbortSignal.timeout(DOWNLOAD_TIMEOUT_MS) });
  if (!response.ok) throw new Error(`${url} answered HTTP ${response.status}`);
  return Buffer.from(await response.arrayBuffer());
}

/** The SHA256SUMS the publish step copies into the package, if it did. */
function shippedSums() {
  try {
    return fs.readFileSync(path.join(__dirname, "..", "SHA256SUMS"), "utf8");
  } catch {
    return undefined;
  }
}

/**
 * Report a launcher failure and return the exit code, keeping the command's contract:
 * `jevgate hook` always exits 0 and says what happened in its JSON reply, since agents
 * read exit 2 as a block; every other command exits 2, a run that could not finish.
 */
function failure(args, message, streams) {
  streams.stderr.write(`jevgate: ${message}\n`);
  if (args[0] !== "hook") return 2;
  const reply = { systemMessage: `JevGate's npm launcher ${message}. Nothing was checked or blocked.` };
  streams.stdout.write(`${JSON.stringify(reply)}\n`);
  return 0;
}

/** Run JevGate with `args`; the exit code. */
async function main(args, host = {}) {
  const {
    env = process.env,
    platform = process.platform,
    arch = process.arch,
    home = os.homedir(),
    streams = process,
    fetchBytes = download,
    sums = shippedSums(),
  } = host;
  const version = require("../package.json").version;
  const build = target(platform, arch);
  let binary;
  try {
    if (!build) throw new Error(`there is no release build for ${platform} ${arch}`);
    binary = await ensureBinary({
      version,
      build,
      cache: cacheDirectory(env, platform, home),
      fetchBytes,
      sums,
      log: (message) => streams.stderr.write(`jevgate: ${message}\n`),
    });
  } catch (error) {
    return failure(args, `could not install JevGate ${version}: ${error.message}. Install it another way: ${INSTALL_PAGE}`, streams);
  }
  const result = spawnSync(binary, args, { stdio: "inherit" });
  if (result.error) return failure(args, `could not start ${binary} (${result.error.message})`, streams);
  if (result.signal) return 128 + (os.constants.signals[result.signal] ?? 0);
  return result.status ?? 2;
}

module.exports = { archiveName, cacheDirectory, checksum, ensureBinary, failure, main, target };
