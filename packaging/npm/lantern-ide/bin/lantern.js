#!/usr/bin/env node
// lantern-ide 실행기: 이 플랫폼용 패키지(lantern-ide-<플랫폼>-<아키텍처>)에 든 바이너리를 그대로 실행한다.
// 의존성과 postinstall 없음. 표준 입출력을 물려주므로 `lantern mcp`(stdio)도 이대로 돈다.
// 플랫폼 목록은 scripts/npm_pack.mjs가 package.json의 optionalDependencies에 채운다.
"use strict";

const { spawnSync } = require("child_process");
const path = require("path");
const { optionalDependencies, version } = require("../package.json");

const SOURCE_BUILD = "cargo install --git https://github.com/Lantern-IDE/lantern lantern-context --locked";

function fail(...lines) {
  console.error(lines.map((l) => `lantern-ide: ${l}`).join("\n"));
  process.exit(1);
}

function usesMusl() {
  if (process.platform !== "linux" || !process.report) return false;
  return !process.report.getReport().header.glibcVersionRuntime;
}

const platform = `${process.platform}-${process.arch}`;
const pkg = `lantern-ide-${platform}`;

if (!(pkg in optionalDependencies)) {
  const known = Object.keys(optionalDependencies).map((n) => n.replace("lantern-ide-", ""));
  fail(`no prebuilt binary for ${platform}.`, `Available: ${known.join(", ")}.`, `To build from source: ${SOURCE_BUILD}`);
}

let dir;
try {
  dir = path.dirname(require.resolve(`${pkg}/package.json`));
} catch {
  if (usesMusl()) {
    fail("the Linux builds need glibc, and this system uses musl (Alpine?).", `To build from source: ${SOURCE_BUILD}`);
  }
  fail(
    `the binary package ${pkg}@${version} is not installed.`,
    "Optional dependencies were probably skipped (--omit=optional), or a lockfile from another OS was used.",
    "Reinstall with: npm install lantern-ide --include=optional",
  );
}

const bin = path.join(dir, "bin", process.platform === "win32" ? "lantern.exe" : "lantern");
const run = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });

if (run.error) fail(`could not run ${bin}: ${run.error.message}`);
if (run.signal) process.kill(process.pid, run.signal);
else process.exit(run.status === null ? 1 : run.status);
