// E2E용 가짜 테스트 명령: 서명 접미사가 '.bad'면 실패한다. 받은 테스트 파일은 출력에 남긴다.
import fs from "node:fs";

const files = process.argv.slice(2);
const src = fs.readFileSync("src/auth/session.ts", "utf8");
console.log(`running ${files.join(" ")}`);
if (src.includes(".bad")) {
  console.log("FAIL testSign: expected signCookie('a') to end with '.sig' but got 'a.bad'");
  process.exit(1);
}
console.log("PASS testSign");
