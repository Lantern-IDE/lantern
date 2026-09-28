// AI 모델 제공자 목록. 첫 실행 화면, 설정의 모델 추가, 모델 고르기가 같이 쓴다.
// Anthropic 말고는 모두 OpenAI 호환 API라 base_url만 다르다.
// 모델 이름은 제공자가 자주 바꾸므로 여기에 적지 않고, 키를 넣은 뒤 제공자의 모델 목록(/models)에서 고르게 한다.

export interface Provider {
  id: string;
  label: string;
  provider: "anthropic" | "openai";
  base_url?: string;
  /** 키를 저장하는 이름 (환경변수 이름과 같다). 없으면 키가 필요 없다 */
  api_key_env?: string;
  key_placeholder?: string;
  /** 키 발급 페이지 */
  key_url?: string;
  /** 모델 목록에서 먼저 골라 둘 모델 (앞의 것부터) */
  prefer?: RegExp[];
  /** 알려진 기본 모델 (목록을 못 받을 때) */
  model?: string;
  price?: [number, number];
  local?: boolean;
}

export const PROVIDERS: Provider[] = [
  { id: "anthropic", label: "Anthropic (Claude)", provider: "anthropic", api_key_env: "ANTHROPIC_API_KEY", key_placeholder: "sk-ant-…", key_url: "https://console.anthropic.com/settings/keys", prefer: [/^claude-opus/, /^claude-sonnet/], model: "claude-opus-5", price: [5, 25] },
  { id: "openai", label: "OpenAI (GPT)", provider: "openai", base_url: "https://api.openai.com/v1", api_key_env: "OPENAI_API_KEY", key_placeholder: "sk-…", key_url: "https://platform.openai.com/api-keys", prefer: [/^gpt-5(?!.*(mini|nano))/, /^gpt-4\.1$/, /^gpt-4o$/] },
  { id: "gemini", label: "Google Gemini", provider: "openai", base_url: "https://generativelanguage.googleapis.com/v1beta/openai", api_key_env: "GEMINI_API_KEY", key_placeholder: "AIza…", key_url: "https://aistudio.google.com/apikey", prefer: [/^gemini-[\d.]+-pro/, /pro/] },
  { id: "deepseek", label: "DeepSeek", provider: "openai", base_url: "https://api.deepseek.com/v1", api_key_env: "DEEPSEEK_API_KEY", key_placeholder: "sk-…", key_url: "https://platform.deepseek.com/api_keys", prefer: [/^deepseek-chat/] },
  { id: "xai", label: "xAI (Grok)", provider: "openai", base_url: "https://api.x.ai/v1", api_key_env: "XAI_API_KEY", key_placeholder: "xai-…", key_url: "https://console.x.ai", prefer: [/^grok-\d(?!.*(mini|fast))/, /^grok/] },
  { id: "mistral", label: "Mistral", provider: "openai", base_url: "https://api.mistral.ai/v1", api_key_env: "MISTRAL_API_KEY", key_url: "https://console.mistral.ai/api-keys", prefer: [/^codestral/, /^mistral-large/] },
  { id: "groq", label: "Groq", provider: "openai", base_url: "https://api.groq.com/openai/v1", api_key_env: "GROQ_API_KEY", key_placeholder: "gsk_…", key_url: "https://console.groq.com/keys", prefer: [/llama-3\.3-70b/, /qwen/] },
  { id: "openrouter", label: "OpenRouter (여러 회사 모델)", provider: "openai", base_url: "https://openrouter.ai/api/v1", api_key_env: "OPENROUTER_API_KEY", key_placeholder: "sk-or-…", key_url: "https://openrouter.ai/keys", prefer: [/^anthropic\/claude/, /^openai\/gpt/] },
  { id: "ollama", label: "Ollama (로컬)", provider: "openai", base_url: "http://localhost:11434/v1", price: [0, 0], local: true },
  { id: "lmstudio", label: "LM Studio (로컬)", provider: "openai", base_url: "http://localhost:1234/v1", price: [0, 0], local: true },
  { id: "custom", label: "기타 OpenAI 호환", provider: "openai", base_url: "" },
];

/** 첫 실행 화면에 보이는 클라우드 제공자 */
export const CLOUD = PROVIDERS.filter((p) => !p.local && p.id !== "custom");

export function provider(id: string): Provider {
  return PROVIDERS.find((p) => p.id === id) ?? PROVIDERS[PROVIDERS.length - 1];
}

/** 설정 파일의 모델 이름. Claude는 예전부터 쓰던 "smart"를 그대로 쓴다 */
export function modelKey(p: Provider): string {
  return p.id === "anthropic" ? "smart" : p.id;
}

// 대화에 쓸 수 없는 모델 (임베딩, 음성, 이미지, 검열 등)
const NOT_CHAT = /(embed|tts|whisper|dall-e|image|imagen|audio|realtime|moderation|transcribe|rerank|guard|ocr|aqa|veo|lyria|computer-use)/i;

/** 제공자가 준 모델 목록에서 대화용만 남기고, 먼저 고를 모델을 앞으로 */
export function chatModels(p: Provider, ids: string[]): { models: string[]; pick: string | null } {
  // Gemini의 OpenAI 호환 목록은 "models/gemini-…" 형식이다
  const models = [...new Set(ids.map((id) => id.replace(/^models\//, "")))].filter((id) => !NOT_CHAT.test(id)).sort();
  let pick: string | null = null;
  for (const re of p.prefer ?? []) {
    // 같은 계열이면 이름이 뒤에 오는(대개 최신) 것
    const hit = models.filter((m) => re.test(m)).sort().pop();
    if (hit) {
      pick = hit;
      break;
    }
  }
  return { models, pick: pick ?? (p.model && models.includes(p.model) ? p.model : models[0] ?? null) };
}
