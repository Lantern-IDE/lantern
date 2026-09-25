// 소스 제어 뷰 (VS Code와 같은 배치): 저장소 목록, 브랜치·원격 동기화, 커밋 메시지, 스테이징된 변경, 변경 사항, 커밋 이력.
// 연 폴더 아래 저장소를 모두 찾아 보여주고(여러 저장소를 담은 작업 폴더), 고른 저장소 하나를 자세히 보여준다.
import { invoke } from "@tauri-apps/api/core";
import { ask } from "./dialog";
import { errorText } from "./api";
import { showContextMenu, type MenuEntry } from "./commands";
import { $, basename, dirname, h, renderDiff } from "./dom";
import { codicon, fileIcon } from "./icons";
import * as editor from "./editor";
import * as toast from "./toast";

interface GitFile { path: string; orig: string | null; index: string; worktree: string }
interface GitStatus { repo: boolean; branch: string | null; ahead: number; behind: number; files: GitFile[] }
export interface Repo { path: string; name: string; branch: string | null; upstream: string | null; ahead: number; behind: number; changes: number; remote: string | null; prefix: string }
interface Commit { hash: string; short: string; author: string; date: string; refs: string[]; subject: string }
interface CommitFile { status: string; path: string; orig: string | null }
interface Branch { name: string; remote: boolean; current: boolean; upstream: string | null; date: string }

const LABEL: Record<string, [string, string]> = {
  M: ["M", "수정됨"],
  A: ["A", "추가됨"],
  D: ["D", "삭제됨"],
  R: ["R", "이름 바뀜"],
  C: ["C", "복사됨"],
  T: ["T", "형식 바뀜"],
  "?": ["U", "추적하지 않음"],
  U: ["!", "충돌"],
};

const HISTORY_PAGE = 30;

let repos: Repo[] = [];
let active = ""; // 고른 저장소 (연 폴더 기준 경로)
let last: GitStatus | null = null;
let restricted = "";
let reposAt = 0;
let busy = false;
// 돌고 있는 동안 들어온 요청. 저장소 목록까지 새로 읽으라는 요청이었는지도 기억한다(잃으면 옛 원격 정보가 남는다).
let pending: { repos: boolean } | null = null;
let historyOpen = false;
let history: Commit[] = [];
let historyMore = false;
const expanded = new Set<string>();
let onCount: (n: number) => void = () => {};
let onBranch: (text: string | null, title: string) => void = () => {};

const repo = () => repos.find((r) => r.path === active) ?? null;

/** 저장소 기준 경로 → 연 폴더 기준 경로 (연 폴더 밖이면 null) */
function projectPath(r: Repo, p: string): string | null {
  if (r.path) return `${r.path}/${p}`;
  if (!r.prefix) return p;
  return p.startsWith(r.prefix) ? p.slice(r.prefix.length) : null;
}

async function action(name: string, paths: string[] = [], message?: string): Promise<string | null> {
  try {
    return await invoke<string>("git_action", { repo: active, action: name, paths, message });
  } catch (e) {
    toast.error(SYNC_FAILED[name] ?? "Git 작업이 실패했습니다", errorText(e));
    return null;
  } finally {
    void refresh({ repos: true });
  }
}

// ── 변경 사항 ────────────────────────────────────────────

function openDiff(f: GitFile, staged: boolean) {
  const r = repo();
  const id = `diff:${staged ? "s" : "w"}:${active}:${f.path}`;
  void editor.closeTab(`page:${id}`).then(() =>
    editor.openPage(id, `${basename(f.path)} (${staged ? "스테이징됨" : "변경"})`, "diff", async (host) => {
      host.classList.add("diff-page");
      host.append(h("div", { class: "diff-head" }, fileIcon(f.path), h("b", {}, r?.path ? `${r.path}/${f.path}` : f.path), h("span", { class: "muted" }, staged ? "스테이징된 변경" : "작업 트리 변경")));
      try {
        const text = await invoke<string>("git_diff", { repo: active, path: f.path, staged });
        host.append(text.trim() ? renderDiff(text) : h("div", { class: "empty-view" }, "바뀐 내용이 없습니다 (줄 끝 문자나 권한만 바뀌었을 수 있습니다)."));
      } catch (e) {
        host.append(h("div", { class: "empty-view" }, errorText(e)));
      }
    }),
  );
}

function iconButton(icon: string, title: string, fn: () => void): HTMLButtonElement {
  const b = h("button", { class: "icon-btn", title, "aria-label": title }, codicon(icon)) as HTMLButtonElement;
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    fn();
  });
  return b;
}

function fileRow(f: GitFile, staged: boolean): HTMLElement {
  const r = repo()!;
  const code = staged ? f.index : f.worktree;
  const [letter, label] = LABEL[code] ?? [code, code];
  const open = projectPath(r, f.path);
  const actions = h("span", { class: "scm-actions" });
  if (open && !f.path.endsWith("/")) actions.append(iconButton("go-to-file", "파일 열기", () => void editor.openFile(open)));
  if (staged) actions.append(iconButton("remove", "스테이징 해제", () => void action("unstage", [f.path])));
  else {
    actions.append(iconButton("discard", "변경 취소", async () => {
      const untracked = f.worktree === "?";
      const ok = await ask(untracked ? `새 파일 '${f.path}'을(를) 휴지통으로 옮길까요?` : `'${f.path}'의 변경을 버리고 마지막 커밋으로 되돌릴까요? 되돌릴 수 없습니다.`, {
        title: "변경 취소", kind: "warning", okLabel: untracked ? "휴지통으로 이동" : "변경 버리기", cancelLabel: "취소",
      });
      if (ok) {
        await action("discard", [f.path]);
        if (open) void editor.reloadIfClean(open);
      }
    }));
    actions.append(iconButton("add", "스테이징", () => void action("stage", [f.path])));
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
  const all = iconButton(staged ? "remove" : "add", staged ? "모두 스테이징 해제" : "모두 스테이징", () => void action(staged ? "unstage" : "stage", files.map((f) => f.path)));
  const head = h("div", { class: "scm-group" }, codicon("chevron-down"), h("span", {}, title), all, h("span", { class: "badge" }, String(files.length)));
  return h("div", {}, head, ...files.map((f) => fileRow(f, staged)));
}

async function commit() {
  const msg = $<HTMLTextAreaElement>("#scm-message");
  if (!last?.files.some((f) => f.index !== " " && f.index !== "?")) {
    const ok = await ask("스테이징된 변경이 없습니다. 모든 변경을 스테이징하고 커밋할까요?", { title: "커밋", okLabel: "모두 스테이징하고 커밋", cancelLabel: "취소" });
    if (!ok) return;
    await action("stage", last?.files.map((f) => f.path) ?? []);
  }
  if (!msg.value.trim()) {
    toast.warn("커밋 메시지를 입력하세요");
    msg.focus();
    return;
  }
  const out = await action("commit", [], msg.value);
  if (out !== null) {
    msg.value = "";
    toast.success("커밋했습니다", out.split("\n")[0]);
    if (historyOpen) void loadHistory(true);
  }
}

// ── 저장소 목록 ──────────────────────────────────────────

function syncText(r: Repo): string {
  return r.ahead || r.behind ? ` ↑${r.ahead} ↓${r.behind}` : "";
}

/** 원격 주소를 짧게: https://host/group/name.git → host/group/name */
function shortRemote(url: string): string {
  return url.replace(/^[a-z+]+:\/\//, "").replace(/^git@([^:]+):/, "$1/").replace(/\.git$/, "");
}

function renderRepos() {
  const box = $("#scm-repos");
  box.classList.toggle("hidden", repos.length < 2);
  if (repos.length < 2) return box.replaceChildren();
  box.replaceChildren(
    h("div", { class: "scm-group" }, codicon("chevron-down"), h("span", {}, "저장소"), h("span", { class: "badge" }, String(repos.length))),
    ...repos.map((r) => {
      const row = h("div", { class: `scm-repo${r.path === active ? " active" : ""}`, role: "button", tabindex: "0", title: `${r.name}${r.remote ? ` · ${r.remote}` : " · 원격 없음"}` },
        codicon("repo"),
        h("span", { class: "rname" }, r.name),
        h("span", { class: "rbranch" }, r.branch ?? ""),
        h("span", { class: "rsync" }, syncText(r).trim()),
        r.changes ? h("span", { class: "badge" }, String(r.changes)) : null);
      const pick = () => {
        if (r.path === active) return;
        active = r.path;
        history = [];
        expanded.clear();
        void refresh();
      };
      row.addEventListener("click", pick);
      row.addEventListener("keydown", (e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          pick();
        }
      });
      return row;
    }),
  );
}

// ── 브랜치와 원격 동기화 ──────────────────────────────────

// 조사가 작업마다 달라서("풀을", "푸시를") 문장째로 둔다
const SYNC_DONE: Record<string, string> = { fetch: "원격에서 가져왔습니다", pull: "원격의 커밋을 받았습니다", push: "원격에 올렸습니다" };
const SYNC_FAILED: Record<string, string> = { fetch: "가져오기에 실패했습니다", pull: "풀에 실패했습니다", push: "푸시에 실패했습니다" };

async function sync(name: "fetch" | "pull" | "push", btn: HTMLElement) {
  btn.classList.add("busy");
  btn.replaceChildren(codicon("loading", "codicon-modifier-spin"));
  try {
    const out = await action(name);
    if (out !== null) {
      const lastLine = out.split("\n").map((l) => l.trim()).filter(Boolean).pop();
      toast.success(SYNC_DONE[name], lastLine && lastLine.length < 160 ? lastLine : undefined);
      if (historyOpen) void loadHistory(true);
    }
  } finally {
    btn.classList.remove("busy");
  }
}

async function branchMenu(anchor: HTMLElement) {
  let list: Branch[];
  try {
    list = await invoke<Branch[]>("git_branches", { repo: active });
  } catch (e) {
    return toast.error("브랜치를 읽지 못했습니다", errorText(e));
  }
  const locals = list.filter((b) => !b.remote);
  const localNames = new Set(locals.map((b) => b.name));
  // 원격 브랜치 중 같은 이름의 로컬 브랜치가 없는 것만 (있으면 로컬로 전환하면 된다)
  const remotes = list.filter((b) => b.remote && !localNames.has(b.name.slice(b.name.indexOf("/") + 1)));
  const switchTo = async (b: Branch) => {
    try {
      await invoke<string>("git_checkout", { repo: active, name: b.name, create: false, remote: b.remote });
      toast.success(`브랜치를 바꿨습니다: ${b.remote ? b.name.slice(b.name.indexOf("/") + 1) : b.name}`);
      history = [];
    } catch (e) {
      toast.error("브랜치를 바꾸지 못했습니다", errorText(e));
    }
    void refresh({ repos: true });
  };
  const items: (MenuEntry | "-")[] = [
    { label: "새 브랜치 만들기…", run: () => newBranchInput() },
    "-",
    ...locals.slice(0, 30).map((b) => ({ label: `${b.current ? "✓ " : "    "}${b.name}${b.upstream ? `  → ${b.upstream}` : ""}`, disabled: b.current, run: () => switchTo(b) })),
  ];
  if (remotes.length) items.push("-", ...remotes.slice(0, 30).map((b) => ({ label: `    ${b.name}  (원격)`, run: () => switchTo(b) })));
  const r = anchor.getBoundingClientRect();
  showContextMenu(r.left, r.bottom + 2, items);
}

function newBranchInput() {
  const bar = $("#scm-sync");
  const input = h("input", { class: "scm-newbranch", placeholder: "새 브랜치 이름 (Enter로 만들기, Esc로 취소)", "aria-label": "새 브랜치 이름", spellcheck: "false" }) as HTMLInputElement;
  bar.replaceChildren(input);
  input.focus();
  let done = false;
  const finish = async (create: boolean) => {
    if (done) return;
    done = true;
    const name = input.value.trim();
    if (create && name) {
      try {
        await invoke<string>("git_checkout", { repo: active, name, create: true, remote: false });
        toast.success(`새 브랜치를 만들었습니다: ${name}`);
      } catch (e) {
        toast.error("브랜치를 만들지 못했습니다", errorText(e));
      }
    }
    void refresh({ repos: true });
  };
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") void finish(true);
    if (e.key === "Escape") void finish(false);
  });
  input.addEventListener("blur", () => void finish(false));
}

function renderSync(r: Repo | null) {
  const bar = $("#scm-sync");
  if (!r || !last?.repo) {
    bar.classList.add("hidden");
    return bar.replaceChildren();
  }
  bar.classList.remove("hidden");
  const branch = h("button", { class: "scm-branch-btn", title: "브랜치 바꾸기·만들기", "aria-label": `브랜치 ${last.branch ?? ""}, 바꾸기` },
    codicon("git-branch"), h("span", {}, last.branch ?? "(브랜치 없음)"), h("span", { class: "rsync" }, syncText({ ...r, ahead: last.ahead, behind: last.behind }).trim()));
  branch.addEventListener("click", () => void branchMenu(branch));
  const remote = r.remote
    ? h("span", { class: "scm-remote", title: r.upstream ? `원격: ${r.remote} · 연결된 브랜치: ${r.upstream}` : `원격: ${r.remote} · 이 브랜치는 아직 원격에 없습니다 (푸시하면 연결됩니다)` }, codicon("cloud"), shortRemote(r.remote))
    : h("span", { class: "scm-remote none", title: "연결된 원격 저장소가 없습니다. 터미널에서 git remote add origin <주소>로 연결하세요." }, "원격 없음");
  const buttons = h("span", { class: "scm-sync-actions" });
  if (r.remote) {
    const mk = (name: "fetch" | "pull" | "push", icon: string, title: string) => {
      const b = iconButton(icon, title, () => void sync(name, b));
      buttons.append(b);
    };
    mk("fetch", "repo-fetch", "가져오기 (git fetch): 원격의 새 커밋을 받아 오되 내 브랜치는 그대로 둡니다");
    mk("pull", "repo-pull", "풀 (git pull --ff-only): 원격의 새 커밋을 내 브랜치에 앞으로 감기로 받습니다");
    mk("push", "repo-push", r.upstream ? "푸시 (git push)" : "푸시: 이 브랜치를 원격에 처음 올리고 연결합니다");
  }
  bar.replaceChildren(branch, remote, buttons);
}

// ── 커밋 이력 ────────────────────────────────────────────

function ago(iso: string): string {
  const s = (Date.now() - new Date(iso).getTime()) / 1000;
  if (!Number.isFinite(s)) return "";
  if (s < 60) return "방금";
  if (s < 3600) return `${Math.floor(s / 60)}분 전`;
  if (s < 86400) return `${Math.floor(s / 3600)}시간 전`;
  if (s < 86400 * 30) return `${Math.floor(s / 86400)}일 전`;
  return new Date(iso).toLocaleDateString();
}

async function loadHistory(reset = false) {
  if (reset) history = [];
  try {
    const page = await invoke<Commit[]>("git_log", { repo: active, skip: history.length, limit: HISTORY_PAGE });
    history = [...history, ...page];
    historyMore = page.length === HISTORY_PAGE;
  } catch (e) {
    toast.error("커밋 이력을 읽지 못했습니다", errorText(e));
  }
  renderBody();
}

function openCommitDiff(c: Commit, f: CommitFile) {
  const id = `commit:${active}:${c.short}:${f.path}`;
  void editor.closeTab(`page:${id}`).then(() =>
    editor.openPage(id, `${basename(f.path)} (${c.short})`, "diff", async (host) => {
      host.classList.add("diff-page");
      host.append(h("div", { class: "diff-head" }, fileIcon(f.path), h("b", {}, f.path), h("span", { class: "muted" }, `${c.short} · ${c.subject}`)));
      try {
        const text = await invoke<string>("git_commit_diff", { repo: active, hash: c.hash, path: f.path });
        host.append(text.trim() ? renderDiff(text) : h("div", { class: "empty-view" }, "바뀐 내용이 없습니다 (이진 파일이거나 권한만 바뀌었을 수 있습니다)."));
      } catch (e) {
        host.append(h("div", { class: "empty-view" }, errorText(e)));
      }
    }),
  );
}

function commitRow(c: Commit): HTMLElement {
  const open = expanded.has(c.hash);
  // 브랜치 표시는 두 개까지 (좁은 사이드바에서 제목을 밀어내지 않게). origin/HEAD는 뺀다.
  const all = c.refs.filter((x) => !/\/HEAD$/.test(x));
  const refs = all.slice(0, 2).map((x) => h("span", { class: `scm-ref${x.startsWith("HEAD") ? " head" : x.startsWith("tag:") ? " tag" : x.includes("/") ? " remote" : ""}` }, x.replace(/^HEAD -> /, "").replace(/^tag: /, "")));
  if (all.length > 2) refs.push(h("span", { class: "scm-ref more" }, `+${all.length - 2}`));
  const row = h("div", { class: `scm-commit-row${open ? " open" : ""}`, role: "button", tabindex: "0", title: `${c.subject}\n${c.short} · ${c.author} · ${new Date(c.date).toLocaleString()}${all.length ? `\n${all.join(", ")}` : ""}` },
    codicon(open ? "chevron-down" : "chevron-right"),
    // 두 줄: 제목 / 브랜치 표시와 시간 (좁은 사이드바에서도 제목이 보이게)
    h("div", { class: "cbody" },
      h("div", { class: "csubject" }, c.subject),
      h("div", { class: "cline" }, ...refs, h("span", { class: "cmeta" }, `${c.author} · ${ago(c.date)}`))));
  const box = h("div", {}, row);
  const toggle = async () => {
    if (expanded.has(c.hash)) {
      expanded.delete(c.hash);
      return renderBody();
    }
    expanded.add(c.hash);
    renderBody();
  };
  row.addEventListener("click", () => void toggle());
  row.addEventListener("keydown", (e) => {
    if (e.key === "Enter") void toggle();
  });
  if (open) {
    const files = h("div", { class: "scm-commit-files" }, h("div", { class: "muted small pad" }, codicon("loading", "codicon-modifier-spin"), " 불러오는 중…"));
    box.append(files);
    void invoke<CommitFile[]>("git_commit_files", { repo: active, hash: c.hash }).then(
      (list) => files.replaceChildren(
        h("div", { class: "cinfo" }, `${c.short} · ${c.author} · ${new Date(c.date).toLocaleString()}`),
        ...(list.length ? list.map((f) => {
          const [letter, label] = LABEL[f.status] ?? [f.status, f.status];
          const fr = h("div", { class: "scm-file", title: `${f.orig ? `${f.orig} → ` : ""}${f.path} · ${label}` },
            fileIcon(f.path), h("span", { class: "fname" }, basename(f.path)), h("span", { class: "dir" }, dirname(f.path)), h("span", { class: `scm-letter s-${letter}` }, letter));
          fr.addEventListener("click", () => openCommitDiff(c, f));
          return fr;
        }) : [h("div", { class: "muted small pad" }, "바뀐 파일이 없습니다 (병합 커밋일 수 있습니다).")]),
      ),
      (e) => files.replaceChildren(h("div", { class: "muted small pad" }, errorText(e))),
    );
  }
  return box;
}

function historySection(): HTMLElement {
  const head = h("div", { class: "scm-group clickable", role: "button", tabindex: "0" }, codicon(historyOpen ? "chevron-down" : "chevron-right"), h("span", {}, "커밋 이력"));
  const toggle = () => {
    historyOpen = !historyOpen;
    if (historyOpen && !history.length) void loadHistory(true);
    else renderBody();
  };
  head.addEventListener("click", toggle);
  head.addEventListener("keydown", (e) => {
    if (e.key === "Enter") toggle();
  });
  if (!historyOpen) return h("div", { class: "scm-history" }, head);
  const more = historyMore ? h("button", { class: "btn btn-ghost block" }, "더 보기") : null;
  more?.addEventListener("click", () => void loadHistory());
  return h("div", { class: "scm-history" }, head,
    ...(history.length ? history.map(commitRow) : [h("div", { class: "muted small pad" }, "아직 커밋이 없습니다.")]),
    more);
}

// ── 그리기 ──────────────────────────────────────────────

function renderBody() {
  const body = $("#scm-body");
  const s = last;
  if (!s?.repo) return;
  const staged = s.files.filter((f) => f.index !== " " && f.index !== "?");
  const changes = s.files.filter((f) => f.worktree !== " ");
  body.replaceChildren(
    ...[group("스테이징된 변경", staged, true), group("변경 사항", changes, false)].filter((x): x is HTMLElement => !!x),
    ...(s.files.length ? [] : [h("div", { class: "empty-view compact" }, "변경 사항이 없습니다.")]),
    historySection(),
  );
}

function renderEmpty(text: string, withInit: boolean) {
  $("#scm-commit").classList.add("hidden");
  $("#scm-sync").classList.add("hidden");
  $("#scm-branch").textContent = "";
  const init = withInit ? h("button", { class: "btn btn-primary block" }, "저장소 만들기 (git init)") : null;
  init?.addEventListener("click", () => void action("init"));
  $("#scm-body").replaceChildren(h("div", { class: "empty-view" }, h("p", {}, text), init));
}

/**
 * 다시 읽는다. 저장소 목록(저장소마다 git을 여러 번 부름)은 `repos: true`이거나 3초가 지났을 때만 새로 찾고,
 * 고른 저장소의 상태는 매번 읽는다 (파일을 저장할 때마다 불린다).
 */
export async function refresh(opts: { repos?: boolean } = {}) {
  if (busy) {
    pending = { repos: !!(pending?.repos || opts.repos) };
    return;
  }
  busy = true;
  try {
    if (opts.repos || Date.now() - reposAt > 3000 || !repos.length) {
      try {
        repos = await invoke<Repo[]>("git_repos");
        restricted = "";
      } catch (e) {
        repos = [];
        restricted = errorText(e);
      }
      reposAt = Date.now();
    }
    if (!repos.some((r) => r.path === active)) {
      // 처음이거나 고른 저장소가 사라졌으면: 변경이 있는 저장소를 먼저
      active = (repos.find((r) => r.changes > 0) ?? repos[0])?.path ?? "";
      history = [];
      expanded.clear();
    }
    renderRepos();
    const total = repos.reduce((n, r) => n + r.changes, 0);
    if (restricted) {
      renderEmpty(restricted, false);
      onCount(0);
      onBranch(null, "");
      return;
    }
    if (!repos.length) {
      renderEmpty("이 폴더와 그 아래 폴더에서 git 저장소를 찾지 못했습니다. 저장소를 만들면 변경 이력을 관리하고 AI가 바꾼 내용을 커밋 단위로 되돌릴 수 있습니다.", true);
      onCount(0);
      onBranch(null, "");
      return;
    }
    const s = await invoke<GitStatus>("git_status", { repo: active }).catch(() => null);
    last = s;
    const r = repo();
    if (r && s) {
      // 고른 저장소는 방금 읽은 상태로 목록도 맞춘다
      Object.assign(r, { branch: s.branch, ahead: s.ahead, behind: s.behind, changes: s.files.length });
      renderRepos();
    }
    $("#scm-commit").classList.toggle("hidden", !s?.repo);
    $("#scm-branch").textContent = repos.length > 1 && r ? r.name : "";
    renderSync(r);
    renderBody();
    onCount(repos.length > 1 ? repos.reduce((n, x) => n + x.changes, 0) : (s?.files.length ?? total));
    onBranch(
      s?.branch ? (repos.length > 1 && r ? `${r.name}: ${s.branch}` : s.branch) + syncText({ ...r!, ahead: s.ahead, behind: s.behind }) : null,
      r?.remote ? `${r.name} · ${r.remote}` : r?.name ?? "",
    );
  } finally {
    busy = false;
    if (pending) {
      const next = pending;
      pending = null;
      void refresh(next);
    }
  }
}

/** 프로젝트를 바꾸면 처음부터 */
export function reset() {
  repos = [];
  active = "";
  last = null;
  history = [];
  historyOpen = false;
  expanded.clear();
  reposAt = 0;
}

export function init(opts: { onCount: (n: number) => void; onBranch: (text: string | null, title: string) => void }) {
  onCount = opts.onCount;
  onBranch = opts.onBranch;
  $("#scm-commit-btn").addEventListener("click", () => void commit());
  $("#scm-message").addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      void commit();
    }
  });
  $("#btn-scm-refresh").addEventListener("click", () => void refresh({ repos: true }));
}
