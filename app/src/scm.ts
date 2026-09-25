// 소스 제어 뷰 (VS Code와 같은 배치): 커밋 메시지, 스테이징된 변경, 변경 사항.
import { invoke } from "@tauri-apps/api/core";
import { ask } from "./dialog";
import { errorText } from "./api";
import { $, basename, dirname, h, renderDiff } from "./dom";
import { codicon, fileIcon } from "./icons";
import * as editor from "./editor";
import * as toast from "./toast";

interface GitFile { path: string; orig: string | null; index: string; worktree: string }
interface GitStatus { repo: boolean; branch: string | null; ahead: number; behind: number; files: GitFile[] }

const LABEL: Record<string, [string, string]> = {
  M: ["M", "수정됨"],
  A: ["A", "추가됨"],
  D: ["D", "삭제됨"],
  R: ["R", "이름 바뀜"],
  C: ["C", "복사됨"],
  "?": ["U", "추적하지 않음"],
  U: ["!", "충돌"],
};

let last: GitStatus | null = null;
let onCount: (n: number) => void = () => {};
let busy = false;

async function run(action: string, paths: string[] = [], message?: string): Promise<string | null> {
  try {
    return await invoke<string>("git_action", { action, paths, message });
  } catch (e) {
    toast.error("Git 작업이 실패했습니다", errorText(e));
    return null;
  } finally {
    void refresh();
  }
}

function openDiff(f: GitFile, staged: boolean) {
  const id = `diff:${staged ? "s" : "w"}:${f.path}`;
  void editor.closeTab(`page:${id}`).then(() =>
    editor.openPage(id, `${basename(f.path)} (${staged ? "스테이징됨" : "변경"})`, "diff", async (host) => {
      host.classList.add("diff-page");
      host.append(h("div", { class: "diff-head" }, fileIcon(f.path), h("b", {}, f.path), h("span", { class: "muted" }, staged ? "스테이징된 변경" : "작업 트리 변경")));
      try {
        const text = await invoke<string>("git_diff", { path: f.path, staged });
        host.append(text.trim() ? renderDiff(text) : h("div", { class: "empty-view" }, "바뀐 내용이 없습니다 (줄 끝 문자나 권한만 바뀌었을 수 있습니다)."));
      } catch (e) {
        host.append(h("div", { class: "empty-view" }, errorText(e)));
      }
    }),
  );
}

function fileRow(f: GitFile, staged: boolean): HTMLElement {
  const code = staged ? f.index : f.worktree;
  const [letter, label] = LABEL[code] ?? [code, code];
  const actions = h("span", { class: "scm-actions" });
  const btn = (icon: string, title: string, fn: () => void) => {
    const b = h("button", { class: "icon-btn", title, "aria-label": title }, codicon(icon));
    b.addEventListener("click", (e) => {
      e.stopPropagation();
      fn();
    });
    actions.append(b);
  };
  if (!f.path.endsWith("/")) btn("go-to-file", "파일 열기", () => void editor.openFile(f.path));
  if (staged) btn("remove", "스테이징 해제", () => void run("unstage", [f.path]));
  else {
    btn("discard", "변경 취소", async () => {
      const untracked = f.worktree === "?";
      const ok = await ask(untracked ? `새 파일 '${f.path}'을(를) 휴지통으로 옮길까요?` : `'${f.path}'의 변경을 버리고 마지막 커밋으로 되돌릴까요? 되돌릴 수 없습니다.`, {
        title: "변경 취소", kind: "warning", okLabel: untracked ? "휴지통으로 이동" : "변경 버리기", cancelLabel: "취소",
      });
      if (ok) {
        await run("discard", [f.path]);
        void editor.reloadIfClean(f.path);
      }
    });
    btn("add", "스테이징", () => void run("stage", [f.path]));
  }
  const row = h("div", { class: "scm-file", title: `${f.path} · ${label}` },
    fileIcon(f.path),
    h("span", { class: "fname" }, basename(f.path)),
    h("span", { class: "dir" }, dirname(f.path)),
    actions,
    h("span", { class: `scm-letter s-${letter}` }, letter));
  row.addEventListener("click", () => openDiff(f, staged));
  return row;
}

function group(title: string, files: GitFile[], staged: boolean): HTMLElement | null {
  if (!files.length) return null;
  const head = h("div", { class: "scm-group" }, codicon("chevron-down"), h("span", {}, title), h("span", { class: "badge" }, String(files.length)));
  const all = h("button", { class: "icon-btn", title: staged ? "모두 스테이징 해제" : "모두 스테이징", "aria-label": staged ? "모두 스테이징 해제" : "모두 스테이징" }, codicon(staged ? "remove" : "add"));
  all.addEventListener("click", (e) => {
    e.stopPropagation();
    void run(staged ? "unstage" : "stage", files.map((f) => f.path));
  });
  head.insertBefore(all, head.lastChild);
  return h("div", {}, head, ...files.map((f) => fileRow(f, staged)));
}

async function commit() {
  const msg = $<HTMLTextAreaElement>("#scm-message");
  if (!last?.files.some((f) => f.index !== " " && f.index !== "?")) {
    const ok = await ask("스테이징된 변경이 없습니다. 모든 변경을 스테이징하고 커밋할까요?", { title: "커밋", okLabel: "모두 스테이징하고 커밋", cancelLabel: "취소" });
    if (!ok) return;
    await run("stage", last?.files.map((f) => f.path) ?? []);
  }
  if (!msg.value.trim()) {
    toast.warn("커밋 메시지를 입력하세요");
    msg.focus();
    return;
  }
  const out = await run("commit", [], msg.value);
  if (out !== null) {
    msg.value = "";
    toast.success("커밋했습니다", out.split("\n")[0]);
  }
}

export async function refresh() {
  if (busy) return;
  busy = true;
  try {
    const s = await invoke<GitStatus>("git_status").catch(() => null);
    last = s;
    const body = $("#scm-body");
    if (!s) {
      body.replaceChildren();
      onCount(0);
      return;
    }
    if (!s.repo) {
      const init = h("button", { class: "btn btn-primary block" }, "저장소 만들기 (git init)");
      init.addEventListener("click", () => void run("init"));
      body.replaceChildren(h("div", { class: "empty-view" }, h("p", {}, "이 폴더는 아직 git 저장소가 아닙니다. 저장소를 만들면 변경 이력을 관리하고 AI가 바꾼 내용을 커밋 단위로 되돌릴 수 있습니다."), init));
      $("#scm-commit").classList.add("hidden");
      onCount(0);
      return;
    }
    $("#scm-commit").classList.remove("hidden");
    const staged = s.files.filter((f) => f.index !== " " && f.index !== "?");
    const changes = s.files.filter((f) => f.worktree !== " ");
    const sync = s.ahead || s.behind ? ` · ↑${s.ahead} ↓${s.behind}` : "";
    $("#scm-branch").textContent = s.branch ? `${s.branch}${sync}` : "";
    body.replaceChildren(
      ...[group("스테이징된 변경", staged, true), group("변경 사항", changes, false)].filter((x): x is HTMLElement => !!x),
      ...(s.files.length ? [] : [h("div", { class: "empty-view" }, "변경 사항이 없습니다.")]),
    );
    onCount(s.files.length);
  } finally {
    busy = false;
  }
}

export function init(opts: { onCount: (n: number) => void }) {
  onCount = opts.onCount;
  $("#scm-commit-btn").addEventListener("click", () => void commit());
  $("#scm-message").addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      void commit();
    }
  });
  $("#btn-scm-refresh").addEventListener("click", () => void refresh());
}
