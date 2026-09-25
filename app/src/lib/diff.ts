// Diff 해석 (순수 함수: 단위 테스트 대상)

/** unified diff에서 파일 경로와 바뀌는 원래 줄 범위 */
export function diffTarget(diff: string): { path: string; ranges: [number, number][] } | null {
  let path = "";
  let isNew = false;
  const lines: number[] = [];
  let old = 0;
  for (const l of diff.split("\n")) {
    if (l.startsWith("--- ")) {
      isNew = l.includes("/dev/null");
      continue;
    }
    if (l.startsWith("+++ ")) {
      path = l.slice(4).trim().replace(/^b\//, "");
      continue;
    }
    const hunk = l.match(/^@@ -(\d+)(?:,\d+)? /);
    if (hunk) {
      old = Number(hunk[1]);
      continue;
    }
    if (!old) continue;
    if (l.startsWith("-")) {
      lines.push(old);
      old++;
    } else if (l.startsWith("+")) lines.push(Math.max(1, old - 1)); // 끼워 넣는 자리 바로 앞 줄
    else if (l.startsWith(" ")) old++;
  }
  if (!path || isNew || !lines.length) return null;
  const sorted = [...new Set(lines)].sort((a, b) => a - b);
  const ranges: [number, number][] = [];
  for (const n of sorted) {
    const last = ranges.at(-1);
    if (last && n <= last[1] + 1) last[1] = n;
    else ranges.push([n, n]);
  }
  return { path, ranges };
}
