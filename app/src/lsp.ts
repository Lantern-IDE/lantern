// 언어 서버 연결. Rust 백엔드가 서버 프로세스의 stdio를 중계하고,
// 여기서는 CodeMirror LSP 클라이언트에 그 통로(Transport)를 붙인다.
import { LSPClient, LSPPlugin, Workspace, languageServerExtensions, type WorkspaceFile } from "@codemirror/lsp-client";
import type { EditorView } from "@codemirror/view";
import type { ChangeSet, Text } from "@codemirror/state";
import DOMPurify from "dompurify";
import { api, errorText, on } from "./api";

let rootUri = "";
const clients = new Map<string, Promise<LSPClient | null>>();
const status = new Map<string, string>();
let displayHandler: (path: string) => Promise<EditorView | null> = async () => null;
let statusHandler: (text: string) => void = () => {};

const handlers = new Map<string, Set<(msg: string) => void>>();
on<{ lang: string; msg: string }>("lsp-msg", ({ lang, msg }) => handlers.get(lang)?.forEach((h) => h(msg)));
/** 이번 세션에서 시작하지 못했거나 죽은 언어 서버. 파일을 열 때마다 다시 띄우고 알리지 않는다. */
const failed = new Set<string>();
on<{ lang: string; reason: string }>("lsp-exit", ({ lang, reason }) => {
  clients.delete(lang);
  if (failed.has(lang)) return;
  failed.add(lang);
  setStatus(lang, "종료됨");
  exitHandler(lang, reason);
});
let exitHandler: (lang: string, reason: string) => void = () => {};
let missingHandler: (lang: string) => void = () => {};

/** 설치한 뒤 다시 시도할 수 있게 실패 기록을 지운다 */
export function retry(lang: string) {
  failed.delete(lang);
  clients.delete(lang);
  status.delete(lang);
}

export function configure(opts: {
  rootUri: string;
  display: (path: string) => Promise<EditorView | null>;
  onStatus: (text: string) => void;
  /** 서버가 예기치 않게 끝났을 때. reason은 서버 stderr의 마지막 줄들 */
  onExit: (lang: string, reason: string) => void;
  /** 서버가 설치되어 있지 않을 때 (설치 제안용) */
  onMissing: (lang: string) => void;
}) {
  missingHandler = opts.onMissing;
  rootUri = opts.rootUri;
  displayHandler = opts.display;
  statusHandler = opts.onStatus;
  exitHandler = opts.onExit;
  clients.clear();
  status.clear();
  failed.clear();
  statusHandler("");
}

export function pathToUri(path: string): string {
  return `${rootUri}/${path.split("/").map(encodeURIComponent).join("/")}`;
}

export function uriToPath(uri: string): string | null {
  if (!uri.startsWith(rootUri + "/")) return null;
  return uri.slice(rootUri.length + 1).split("/").map(decodeURIComponent).join("/");
}

function setStatus(lang: string, text: string) {
  status.set(lang, text);
  statusHandler([...status].map(([l, s]) => `${l} ${s}`).join(" · "));
}

/** 편집기 하나 = 파일 하나. 다른 파일로 이동하면 탭을 연다. */
class LanternWorkspace extends Workspace {
  files: (WorkspaceFile & { view: EditorView })[] = [];
  private versions: Record<string, number> = {};

  private nextVersion(uri: string) {
    return (this.versions[uri] = (this.versions[uri] ?? -1) + 1);
  }

  syncFiles(): ReturnType<Workspace["syncFiles"]> {
    const result: { changes: ChangeSet; file: WorkspaceFile; prevDoc: Text }[] = [];
    for (const file of this.files) {
      const plugin = LSPPlugin.get(file.view);
      if (!plugin) continue;
      const changes = plugin.unsyncedChanges;
      if (!changes.empty) {
        result.push({ changes, file, prevDoc: file.doc });
        file.doc = file.view.state.doc;
        file.version = this.nextVersion(file.uri);
        plugin.clear();
      }
    }
    return result;
  }

  openFile(uri: string, languageId: string, view: EditorView): void {
    if (this.getFile(uri)) return;
    const file = {
      uri,
      languageId,
      version: this.nextVersion(uri),
      doc: view.state.doc as Text,
      view,
      getView: () => view,
    };
    this.files.push(file);
    this.client.didOpen(file);
  }

  closeFile(uri: string): void {
    const file = this.getFile(uri);
    if (!file) return;
    this.files = this.files.filter((f) => f !== file);
    this.client.didClose(uri);
  }

  async displayFile(uri: string): Promise<EditorView | null> {
    const path = uriToPath(uri);
    return path === null ? null : displayHandler(path);
  }
}

async function start(lang: string): Promise<LSPClient | null> {
  setStatus(lang, "시작 중…");
  try {
    await api.lspStart(lang);
  } catch (e) {
    setStatus(lang, "없음");
    failed.add(lang);
    const msg = errorText(e);
    console.info(`언어 서버 ${lang}: ${msg}`);
    if (msg.includes("설치되어 있지 않습니다")) missingHandler(lang);
    return null;
  }
  const set = new Set<(msg: string) => void>();
  handlers.set(lang, set);
  const client = new LSPClient({
    rootUri,
    extensions: languageServerExtensions(),
    sanitizeHTML: (html) => DOMPurify.sanitize(html),
    timeout: 10_000,
    workspace: (c) => new LanternWorkspace(c),
  });
  client.connect({
    send: (msg) => {
      api.lspSend(lang, msg).catch(() => {});
    },
    subscribe: (h) => set.add(h),
    unsubscribe: (h) => set.delete(h),
  });
  // 프로세스가 떠도 초기화에서 실패할 수 있다 (예: TypeScript 본체를 못 찾음). 성공해야 ✓
  try {
    await client.initializing;
  } catch (e) {
    const msg = (e as { message?: string })?.message ?? errorText(e);
    setStatus(lang, "오류");
    failed.add(lang);
    exitHandler(lang, `초기화 실패: ${msg}`);
    return null;
  }
  setStatus(lang, "✓");
  return client;
}

interface Location { uri: string; range: { start: { line: number; character: number } } }

/**
 * 참조 찾기. 이미 켜져 있는 언어 서버에만 묻는다 (이것 때문에 서버를 새로 띄우지 않는다).
 * 줄·열은 0부터. 결과는 프로젝트 기준 경로와 1부터 세는 줄. 서버가 없거나 늦으면 null.
 */
export async function references(lang: string, path: string, line: number, character: number): Promise<{ path: string; line: number }[] | null> {
  if (failed.has(lang)) return null;
  const pending = clients.get(lang);
  if (!pending) return null;
  const client = await pending;
  if (!client) return null;
  const req = client.request<unknown, Location[] | null>("textDocument/references", {
    textDocument: { uri: pathToUri(path) },
    position: { line, character },
    context: { includeDeclaration: false },
  });
  const timeout = new Promise<null>((r) => setTimeout(() => r(null), 5000));
  try {
    const res = await Promise.race([req, timeout]);
    if (!res) return null;
    return res.map((l) => ({ path: uriToPath(l.uri), line: l.range.start.line + 1 })).filter((x): x is { path: string; line: number } => x.path !== null);
  } catch {
    return null;
  }
}

/** 언어별 클라이언트. 처음 요청할 때 서버를 띄운다. 설치되어 있지 않으면 null. */
export function clientFor(lang: string): Promise<LSPClient | null> {
  if (failed.has(lang)) return Promise.resolve(null);
  let c = clients.get(lang);
  if (!c) {
    c = start(lang);
    clients.set(lang, c);
  }
  return c;
}
