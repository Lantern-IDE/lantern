// 업데이트 정보 파일(latest.json) 생성기.
// 릴리스 빌드(tauri.release.conf.json) 뒤에 실행하면 설치 파일의 서명(.sig)을 읽어
// target/release/bundle/nsis/latest.json 을 만든다. 이 파일을 설치 파일과 함께 GitHub 릴리스에 올린다.
// 사용: node scripts/latest_json.mjs [--notes "변경 내용"]

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const REPO = "Lantern-IDE/lantern";
const argv = process.argv.slice(2);
const i = argv.indexOf("--notes");
const notes = i >= 0 ? argv[i + 1] ?? "" : "";

const conf = JSON.parse(fs.readFileSync(path.join(ROOT, "app/src-tauri/tauri.conf.json"), "utf8"));
const version = conf.version;
const dir = path.join(ROOT, "target/release/bundle/nsis");
const setup = `${conf.productName}_${version}_x64-setup.exe`;
const sig = path.join(dir, `${setup}.sig`);
if (!fs.existsSync(sig)) {
  console.error(`${sig} 가 없습니다. 서명 키를 설정하고 릴리스 빌드를 먼저 하세요 (docs/배포_가이드.md 2절).`);
  process.exit(1);
}

const latest = {
  version,
  notes,
  pub_date: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
  platforms: {
    "windows-x86_64": {
      signature: fs.readFileSync(sig, "utf8").trim(),
      url: `https://github.com/${REPO}/releases/download/v${version}/${setup}`,
    },
  },
};
fs.writeFileSync(path.join(dir, "latest.json"), JSON.stringify(latest, null, 2) + "\n");
console.log(`latest.json 생성: 버전 ${version} → ${latest.platforms["windows-x86_64"].url}`);
