// 백엔드(Rust) 명령과 이벤트의 타입이 있는 얇은 래퍼.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface ProjectInfo { root: string; root_uri: string; name: string; trusted: boolean; migrated: boolean }
export interface Entry { name: string; path: string; is_dir: boolean }
export interface AgentDef { id: string; name: string; description: string; model: string; tools: string[]; path: string | null }
export interface MonthUsage { cost_usd: number; input_tokens: number; output_tokens: number; requests: number }
export interface ModelInfo {
  default: string;
  models: { key: string; provider: string; model: string; has_key: boolean }[];
  month: string;
  usage: MonthUsage;
  limit_usd: number;
  warn_at_percent: number;
}
export interface Usage { input_tokens: number; output_tokens: number; cache_read_tokens: number; cache_write_tokens: number }

export type AgentEvent = { session: string } & (
  | { kind: "request"; request_id: number; agent: string; model_key: string; model: string; system: string; context: string; context_tokens: number; tools: string[] }
  | { kind: "text"; text: string }
  | { kind: "tool_start"; name: string }
  | { kind: "tool_call"; id: string; name: string; input: Record<string, unknown> }
  | { kind: "tool_result"; id: string; name: string; content: string; is_error: boolean }
  | { kind: "approval"; id: string; approval_kind: "edit" | "command"; title: string; detail: string }
  | { kind: "approval_resolved"; id: string; approved: boolean }
  | { kind: "usage"; request_id: number; model: string; usage: Usage; cost_usd: number | null; month_cost_usd: number; month_limit_usd: number }
  | { kind: "done"; changed: string[]; checkpoint: string | null }
  | { kind: "error"; message: string }
);

export interface IndexEvent {
  status: "running" | "done" | "error";
  message?: string;
  stats?: { scanned: number; indexed: number; unchanged: number; removed: number; elapsed_ms: number };
}

export interface SettingsModel {
  provider: string;
  model: string;
  base_url?: string | null;
  api_key_env?: string | null;
  max_tokens: number;
  effort?: string | null;
  price_input?: number | null;
  price_output?: number | null;
  key_source: "env" | "keychain" | "config" | null;
  needs_key: boolean;
}
export interface SettingsSnapshot {
  config: {
    models: Record<string, SettingsModel>;
    routing: { default: string; completion?: string };
    budget: { monthly_usd_limit: number; warn_at_percent: number };
    context: { budget_tokens: number };
    agent: { max_steps: number; auto_approve: string[]; allowed_commands: string[] };
    hooks: { on_save: string[]; on_agent_done: string[] };
  };
  global_path: string | null;
  global_exists: boolean;
  project_path: string | null;
  project_exists: boolean;
}
export interface TestResult { ok: boolean; message: string; ms: number; models: string[] }
export interface LocalServer { name: string; base_url: string; running: boolean; models: string[] }

export const api = {
  openProject: (path: string) => invoke<ProjectInfo>("open_project", { path }),
  startupPath: () => invoke<string | null>("startup_path"),
  setTrust: (trusted: boolean) => invoke<void>("set_trust", { trusted }),
  listDir: (path: string) => invoke<Entry[]>("list_dir", { path }),
  listFiles: () => invoke<string[]>("list_files"),
  readFile: (path: string) => invoke<string>("read_file", { path }),
  writeFile: (path: string, content: string) => invoke<void>("write_file", { path, content }),
  ensureConfig: () => invoke<string>("ensure_config"),
  getSettings: () => invoke<SettingsSnapshot>("get_settings"),
  setSettings: (scope: "global" | "project", changes: [string, unknown][]) => invoke<void>("set_settings", { scope, changes }),
  setApiKey: (modelKey: string, key: string) => invoke<void>("set_api_key", { modelKey, key }),
  deleteApiKey: (modelKey: string) => invoke<void>("delete_api_key", { modelKey }),
  testModel: (modelKey: string) => invoke<TestResult>("test_model", { modelKey }),
  probeLocal: () => invoke<LocalServer[]>("probe_local"),
  listAgents: () => invoke<AgentDef[]>("list_agents"),
  modelInfo: () => invoke<ModelInfo>("model_info"),
  agentSend: (session: string, agent: string, text: string, file: string | null, line: number | null) =>
    invoke<void>("agent_send", { session, agent, text, file, line }),
  agentCancel: (session: string) => invoke<void>("agent_cancel", { session }),
  agentReset: (session: string) => invoke<void>("agent_reset", { session }),
  revertCheckpoint: (id: string, force = false) => invoke<number>("revert_checkpoint", { id, force }),
  resolveApproval: (session: string, id: string, approved: boolean) =>
    invoke<void>("resolve_approval", { session, id, approved }),
  contextPreview: (query: string, file: string | null, line: number | null) =>
    invoke<{ markdown: string; used_tokens: number; elapsed_ms: number; items: number }>("context_preview", { query, file, line }),
  termSpawn: (cols: number, rows: number) => invoke<number>("term_spawn", { cols, rows }),
  termWrite: (id: number, data: string) => invoke<void>("term_write", { id, data }),
  termResize: (id: number, cols: number, rows: number) => invoke<void>("term_resize", { id, cols, rows }),
  termKill: (id: number) => invoke<void>("term_kill", { id }),
  lspStart: (lang: string) => invoke<void>("lsp_start", { lang }),
  lspSend: (lang: string, msg: string) => invoke<void>("lsp_send", { lang, msg }),
  lspInstallPlan: (lang: string, ignoreFound = false) =>
    invoke<{ summary: string; missing: string | null } | null>("lsp_install_plan", { lang, ignoreFound }),
  lspInstall: (lang: string) => invoke<void>("lsp_install", { lang }),
};

export function on<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (e) => handler(e.payload));
}

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
