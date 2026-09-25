// 단축키 문자열 다루기 (화면과 무관한 순수 함수: 단위 테스트 대상)

export const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

interface KeyLike { key: string; code: string; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean; altKey: boolean }

/** 키 이벤트 → "Ctrl+Shift+K" 형식. 수정 키만 눌렀거나 수정 키 없는 일반 글자면 null (macOS의 ⌘는 Ctrl로 적는다) */
export function fromEvent(e: KeyLike): string | null {
  if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return null;
  let main: string;
  if (e.code.startsWith("Key")) main = e.code.slice(3);
  else if (e.code.startsWith("Digit")) main = e.code.slice(5);
  else if (e.code === "Backquote") main = "`";
  else if (/^F\d+$/.test(e.key)) main = e.key;
  else main = e.key.length === 1 ? e.key.toUpperCase() : e.key;
  const mods = [e.ctrlKey || e.metaKey ? "Ctrl" : "", e.shiftKey ? "Shift" : "", e.altKey ? "Alt" : ""].filter(Boolean);
  // 수정 키 없는 일반 글자는 입력과 겹치므로 받지 않는다 (F키, Escape 등은 허용)
  if (!mods.length && main.length === 1) return null;
  return [...mods, main].join("+");
}

/** 화면에 보이는 단축키. macOS에서는 ⌃⌥⇧⌘ 기호로 (저장 형식은 그대로 Ctrl+…) */
export function displayKey(key: string, mac = isMac): string {
  if (!mac) return key;
  const parts = key.split("+");
  const main = parts.pop() ?? "";
  const sym: Record<string, string> = { Ctrl: "⌘", Shift: "⇧", Alt: "⌥" };
  // macOS 관례 순서: ⌥ ⇧ ⌘
  const order = ["Alt", "Shift", "Ctrl"];
  const mods = order.filter((m) => parts.includes(m)).map((m) => sym[m]).join("");
  return `${mods}${main.length === 1 ? main.toUpperCase() : main}`;
}
