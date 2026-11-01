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

/** 이보다 긴 줄은 줄 안 강조를 건너뛴다 (압축된 코드 등) */
const SPAN_MAX = 2000;

/** 바뀐 두 줄에서 공통 앞뒤를 뺀 가운데 [시작, 끝). 공백 말고 같은 데가 없으면 줄 전체가 바뀐 것이라 null */
function middle(a: string, b: string): { a: [number, number]; b: [number, number] } | null {
  if (a.length > SPAN_MAX || b.length > SPAN_MAX) return null;
  const max = Math.min(a.length, b.length);
  let pre = 1; // 맨 앞 +/-는 뺀다
  while (pre < max && a[pre] === b[pre]) pre++;
  let suf = 0;
  while (suf < max - pre && a[a.length - 1 - suf] === b[b.length - 1 - suf]) suf++;
  // 이모지 같은 서로게이트 쌍을 가르지 않게
  if (pre > 1 && a.charCodeAt(pre - 1) >> 10 === 0x36) pre--;
  if (suf > 0 && a.charCodeAt(a.length - suf) >> 10 === 0x37) suf--;
  if (!a.slice(1, pre).trim() && !a.slice(a.length - suf).trim()) return null;
  return { a: [pre, a.length - suf], b: [pre, b.length - suf] };
}

/** 짝지을 수 있는 -/+ 줄의 달라진 곳: 줄 번호 → [시작, 끝). 빈 범위(넣기만 한 줄의 짝)는 뺀다 */
export function changedSpans(lines: string[]): Map<number, [number, number]> {
  const out = new Map<number, [number, number]>();
  const isDel = (l: string) => l.startsWith("-") && !l.startsWith("---");
  const isAdd = (l: string) => l.startsWith("+") && !l.startsWith("+++");
  for (let i = 0; i < lines.length;) {
    if (!isDel(lines[i])) {
      i++;
      continue;
    }
    let dels = i;
    while (dels < lines.length && isDel(lines[dels])) dels++;
    let adds = dels;
    while (adds < lines.length && isAdd(lines[adds])) adds++;
    // 줄 수가 다르면 어느 줄끼리 짝인지 알 수 없다
    if (adds - dels === dels - i) {
      for (let k = 0; k < dels - i; k++) {
        const m = middle(lines[i + k], lines[dels + k]);
        if (!m) continue;
        if (m.a[0] < m.a[1]) out.set(i + k, m.a);
        if (m.b[0] < m.b[1]) out.set(dels + k, m.b);
      }
    }
    i = adds;
  }
  return out;
}
