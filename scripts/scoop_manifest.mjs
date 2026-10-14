// Scoop 매니페스트(bucket/lantern.json)를 게시한 릴리스로 만든다.
// 사용: node scripts/scoop_manifest.mjs v0.1.5      (릴리스가 게시된 뒤. 설치 파일을 받아 SHA-256을 잰다)
//
// 설치 파일(NSIS)을 7-Zip으로 풀어 쓰는 휴대용 설치다. 앱 안의 자동 업데이트 대신 `scoop update lantern`으로 올린다.

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const tag = process.argv[2];
if (!/^v\d+\.\d+\.\d+$/.test(tag ?? "")) {
  console.error("사용: node scripts/scoop_manifest.mjs v0.1.5");
  process.exit(2);
}
const version = tag.slice(1);
const asset = `Lantern_${version}_x64-setup.exe`;
const url = `https://github.com/Lantern-IDE/lantern/releases/download/${tag}/${asset}`;

const res = await fetch(url);
if (!res.ok) {
  console.error(`${url} 받기 실패 (${res.status}). 릴리스가 게시됐는지 확인하세요 (초안은 받을 수 없음)`);
  process.exit(1);
}
const bytes = Buffer.from(await res.arrayBuffer());
const hash = crypto.createHash("sha256").update(bytes).digest("hex");

const manifest = {
  version,
  description: "Lightweight AI code editor with a local context engine, impact radius and a code map",
  homepage: "https://lantern-ide.github.io",
  license: "Apache-2.0",
  notes: ["Lantern does not update itself when installed with Scoop. Use `scoop update lantern`.", "Scoop으로 설치하면 Lantern이 스스로 업데이트하지 않습니다. `scoop update lantern`으로 받으세요."],
  architecture: {
    "64bit": { url: `${url}#/dl.7z`, hash },
  },
  // NSIS 설치 파일을 풀면 설치 프로그램 부속물이 함께 나온다
  pre_install: ["Remove-Item \"$dir\\`$PLUGINSDIR\", \"$dir\\uninstall.exe\" -Recurse -Force -ErrorAction SilentlyContinue"],
  bin: [["lantern-app.exe", "lantern-ide"]],
  shortcuts: [["lantern-app.exe", "Lantern"]],
  checkver: { github: "https://github.com/Lantern-IDE/lantern" },
  autoupdate: {
    architecture: {
      "64bit": { url: "https://github.com/Lantern-IDE/lantern/releases/download/v$version/Lantern_$version_x64-setup.exe#/dl.7z" },
    },
  },
};

const out = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "bucket", "lantern.json");
fs.mkdirSync(path.dirname(out), { recursive: true });
fs.writeFileSync(out, JSON.stringify(manifest, null, 2) + "\n");
console.log(`${out}: ${version} · ${hash}`);
