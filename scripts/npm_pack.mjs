// npm 패키지 조립: CLI 릴리스 파일을 SHA256SUMS로 확인한 뒤 npm에 올릴 패키지 폴더를 만든다.
// 사용: node scripts/npm_pack.mjs <릴리스 폴더> <버전> [--out <폴더>] [--allow-missing]
//   --out            결과 폴더 (기본: <릴리스 폴더>/npm)
//   --allow-missing  없는 플랫폼을 건너뛴다. 로컬 확인용이고 게시에는 쓰지 않는다
//
// 결과는 lantern-ide와 플랫폼 패키지(lantern-ide-win32-x64 등) 폴더다. 각 폴더에서 `npm pack` 한다.
// Windows에서 npm pack을 하면 macOS·Linux 바이너리의 실행 권한이 사라지므로, 게시는 npm-publish.yml(Linux)로만 한다.
//
// 압축은 외부 도구 없이 푼다. tar는 Windows에서 PATH에 따라 종류가 달라지고, 리눅스 tar는 zip을 못 푼다.

import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import zlib from "node:zlib";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const MAIN = "lantern-ide";

// optional: 호스트 러너에서 빌드가 보장되지 않아 없어도 실패하지 않는 플랫폼 (cli-release.yml과 같은 표)
const PLATFORMS = [
  { target: "x86_64-pc-windows-msvc", os: "win32", cpu: "x64" },
  { target: "aarch64-pc-windows-msvc", os: "win32", cpu: "arm64", optional: true },
  { target: "x86_64-apple-darwin", os: "darwin", cpu: "x64" },
  { target: "aarch64-apple-darwin", os: "darwin", cpu: "arm64" },
  { target: "x86_64-unknown-linux-gnu", os: "linux", cpu: "x64", libc: "glibc" },
  { target: "aarch64-unknown-linux-gnu", os: "linux", cpu: "arm64", libc: "glibc" },
];

function die(message) {
  console.error(message);
  process.exit(1);
}

function parseArgs(argv) {
  const opts = { allowMissing: false, out: null, positional: [] };
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--allow-missing") opts.allowMissing = true;
    else if (argv[i] === "--out") opts.out = argv[++i];
    else opts.positional.push(argv[i]);
  }
  const [dir, version] = opts.positional;
  if (opts.positional.length !== 2 || !/^\d+\.\d+\.\d+$/.test(version)) {
    console.error("사용: node scripts/npm_pack.mjs <릴리스 폴더> <버전 예: 0.1.0> [--out <폴더>] [--allow-missing]");
    process.exit(2);
  }
  return { ...opts, dir: path.resolve(dir), version, out: path.resolve(opts.out ?? path.join(dir, "npm")) };
}

const sha256 = (bytes) => crypto.createHash("sha256").update(bytes).digest("hex");

/** SHA256SUMS의 모든 파일을 확인하고, 목록에 없는 릴리스 파일이 있으면 멈춘다. 파일 이름 → 내용을 돌려준다 */
function verifiedFiles(dir) {
  const sumsPath = path.join(dir, "SHA256SUMS");
  if (!fs.existsSync(sumsPath)) die(`${sumsPath} 가 없습니다. 릴리스 파일을 모두 받았는지 확인하세요.`);
  const files = new Map();
  for (const line of fs.readFileSync(sumsPath, "utf8").split(/\r?\n/)) {
    if (!line.trim()) continue;
    const m = /^([0-9a-fA-F]{64}) [ *]([^/\\]+)$/.exec(line);
    if (!m) die(`SHA256SUMS 줄을 읽을 수 없습니다: ${line}`);
    const [, want, name] = m;
    const file = path.join(dir, name);
    if (!fs.existsSync(file)) die(`SHA256SUMS에 있는 파일이 없습니다: ${name}`);
    const bytes = fs.readFileSync(file);
    const got = sha256(bytes);
    if (got !== want.toLowerCase()) die(`체크섬이 다릅니다: ${name}\n  SHA256SUMS: ${want.toLowerCase()}\n  실제:       ${got}`);
    files.set(name, bytes);
  }
  for (const name of fs.readdirSync(dir)) {
    if (name.startsWith("lantern-") && !files.has(name)) die(`SHA256SUMS에 없는 릴리스 파일입니다: ${name}`);
  }
  return files;
}

/** zip 안의 파일. 크기는 중앙 디렉터리에서 읽는다 (Compress-Archive는 지역 헤더 크기를 비울 수 있다) */
function readZip(buf) {
  let end = buf.length - 22;
  while (end >= 0 && buf.readUInt32LE(end) !== 0x06054b50) end--;
  if (end < 0) throw new Error("zip 끝 표시가 없습니다");
  const count = buf.readUInt16LE(end + 10);
  let p = buf.readUInt32LE(end + 16);
  const files = new Map();
  for (let i = 0; i < count; i++) {
    if (buf.readUInt32LE(p) !== 0x02014b50) throw new Error("zip 중앙 디렉터리가 깨졌습니다");
    const method = buf.readUInt16LE(p + 10);
    const size = buf.readUInt32LE(p + 20);
    const [nameLen, extraLen, commentLen] = [buf.readUInt16LE(p + 28), buf.readUInt16LE(p + 30), buf.readUInt16LE(p + 32)];
    const local = buf.readUInt32LE(p + 42);
    const name = buf.toString("utf8", p + 46, p + 46 + nameLen);
    const start = local + 30 + buf.readUInt16LE(local + 26) + buf.readUInt16LE(local + 28);
    const raw = buf.subarray(start, start + size);
    if (method !== 0 && method !== 8) throw new Error(`지원하지 않는 zip 압축 방식 ${method}: ${name}`);
    files.set(name, method === 8 ? zlib.inflateRawSync(raw) : raw);
    p += 46 + nameLen + extraLen + commentLen;
  }
  return files;
}

/** tar.gz 안의 일반 파일. 긴 이름 머리글은 쓰지 않는다 (짧은 이름 세 개뿐이다) */
function readTarGz(buf) {
  const tar = zlib.gunzipSync(buf);
  const field = (start, len) => tar.toString("latin1", start, start + len).split("\0")[0];
  const files = new Map();
  for (let p = 0; p + 512 <= tar.length; ) {
    const name = field(p, 100);
    if (!name) break;
    const size = parseInt(field(p + 124, 12).trim(), 8);
    const type = String.fromCharCode(tar[p + 156]);
    if (type === "0" || type === "\0") files.set(name.replace(/^\.\//, ""), tar.subarray(p + 512, p + 512 + size));
    p += 512 + Math.ceil(size / 512) * 512;
  }
  return files;
}

const pkgName = (p) => `${MAIN}-${p.os}-${p.cpu}`;

function writePlatformPackage(p, archive, version, out) {
  const exe = p.os === "win32" ? "lantern.exe" : "lantern";
  const binary = archive.get(exe);
  if (!binary) die(`${p.target} 압축 파일에 ${exe}가 없습니다.`);

  const dir = path.join(out, pkgName(p));
  fs.mkdirSync(path.join(dir, "bin"), { recursive: true });
  fs.writeFileSync(path.join(dir, "bin", exe), binary, { mode: 0o755 });
  for (const doc of ["LICENSE", "THIRD_PARTY_NOTICES.md"]) {
    fs.writeFileSync(path.join(dir, doc), archive.get(doc) ?? fs.readFileSync(path.join(ROOT, doc)));
  }
  fs.writeFileSync(path.join(dir, "README.md"), `# ${pkgName(p)}\n\nThe \`lantern\` binary for ${p.os}-${p.cpu}. Install [lantern-ide](https://www.npmjs.com/package/lantern-ide) instead of this package.\n`);
  const manifest = {
    name: pkgName(p),
    version,
    description: `Lantern CLI binary for ${p.os}-${p.cpu}. Install lantern-ide instead.`,
    license: "Apache-2.0",
    repository: { type: "git", url: "git+https://github.com/Lantern-IDE/lantern.git" },
    os: [p.os],
    cpu: [p.cpu],
    ...(p.libc ? { libc: [p.libc] } : {}),
    files: ["bin", "THIRD_PARTY_NOTICES.md"],
    preferUnplugged: true,
  };
  fs.writeFileSync(path.join(dir, "package.json"), JSON.stringify(manifest, null, 2) + "\n");
  return path.join(dir, "bin", exe);
}

function writeMainPackage(names, version, out) {
  const dir = path.join(out, MAIN);
  fs.cpSync(path.join(ROOT, "packaging/npm", MAIN), dir, { recursive: true });
  fs.copyFileSync(path.join(ROOT, "LICENSE"), path.join(dir, "LICENSE"));
  const manifest = JSON.parse(fs.readFileSync(path.join(dir, "package.json"), "utf8"));
  manifest.version = version;
  manifest.optionalDependencies = Object.fromEntries(names.map((n) => [n, version]));
  fs.writeFileSync(path.join(dir, "package.json"), JSON.stringify(manifest, null, 2) + "\n");
}

/** 이 컴퓨터에서 돌 수 있는 바이너리가 있으면 판을 맞춰 본다 */
function checkHostBinary(binaries, version) {
  const host = binaries.find(({ p }) => p.os === process.platform && p.cpu === process.arch);
  if (!host) return console.log(`판 확인 건너뜀: ${process.platform}-${process.arch} 바이너리가 없습니다.`);
  const run = spawnSync(host.bin, ["--version"], { encoding: "utf8" });
  // musl이나 오래된 glibc처럼 이 컴퓨터에서 못 도는 경우. 실제 실행 확인은 npm-publish.yml이 한다
  if (run.error || run.status !== 0) return console.warn(`판 확인 건너뜀: 이 컴퓨터에서 ${host.bin}을 실행하지 못합니다.`);
  const got = run.stdout.trim();
  if (got !== `lantern ${version}`) die(`판이 다릅니다: 바이너리는 '${got}', 요청한 판은 ${version}입니다.`);
  console.log(`판 확인: ${got}`);
}

function main() {
  const { dir, version, out, allowMissing } = parseArgs(process.argv.slice(2));
  const files = verifiedFiles(dir);
  console.log(`체크섬 확인: ${files.size}개 파일`);

  const binaries = [];
  for (const p of PLATFORMS) {
    fs.rmSync(path.join(out, pkgName(p)), { recursive: true, force: true });
    const isZip = p.os === "win32";
    const name = `lantern-${p.target}.${isZip ? "zip" : "tar.gz"}`;
    const bytes = files.get(name);
    if (!bytes) {
      if (!p.optional && !allowMissing) die(`필수 파일이 SHA256SUMS에 없습니다: ${name} (로컬 확인이면 --allow-missing)`);
      console.log(`건너뜀: ${name} 없음`);
      continue;
    }
    binaries.push({ p, bin: writePlatformPackage(p, (isZip ? readZip : readTarGz)(bytes), version, out) });
  }
  if (binaries.length === 0) die("플랫폼 파일이 하나도 없습니다.");
  checkHostBinary(binaries, version);

  fs.rmSync(path.join(out, MAIN), { recursive: true, force: true });
  writeMainPackage(binaries.map(({ p }) => pkgName(p)), version, out);

  if (process.platform === "win32" && binaries.some(({ p }) => p.os !== "win32")) {
    console.warn("주의: Windows에서 npm pack을 하면 macOS·Linux 바이너리의 실행 권한이 사라집니다. 게시는 npm-publish.yml로 하세요.");
  }
  console.log(`${out}: ${MAIN}@${version}, 플랫폼 ${binaries.length}개 (${binaries.map(({ p }) => `${p.os}-${p.cpu}`).join(", ")})`);
}

main();
