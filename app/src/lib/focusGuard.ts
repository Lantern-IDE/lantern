// 결정 버튼이 입력을 가로채지 않게 한다. 치는 중에 초점이 '적용'으로 옮겨 가면
// 치던 Enter·Space가 그대로 버튼을 눌러 보지 못한 변경이 적용된다.

/** 마지막 키 입력 뒤 이만큼은 초점을 옮기지 않는다 */
const QUIET_MS = 1500;
let lastKey = -Infinity;

/** 앱을 켤 때 한 번. 다른 처리기가 전파를 막아도 알도록 맨 앞에서 듣는다 */
export function watchKeys() {
  window.addEventListener("keydown", () => (lastKey = performance.now()), true);
}

/** 글자를 받는 곳: 입력칸, 편집기(CodeMirror), 터미널 */
export function isEditable(el: Element | null): boolean {
  return !!el?.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"]), .cm-editor, .xterm');
}

/** 결정 버튼으로 초점을 옮겨도 되는가. 키보드만 쓰는 사람은 이 이동에 기댄다 */
export function mayTakeFocus(now = performance.now()): boolean {
  return !isEditable(document.activeElement) && now - lastKey >= QUIET_MS;
}
