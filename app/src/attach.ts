// 채팅에 이미지 붙이기: 붙여넣기·끌어 놓기·버튼. 큰 이미지는 보내기 전에 줄인다.
import { $, h } from "./dom";
import { codicon } from "./icons";
import * as toast from "./toast";

export interface Image { mediaType: string; data: string }
interface Pending extends Image { url: string }

const MAX = 4;
/** 공급자들이 권하는 긴 변 (더 크면 어차피 줄여서 본다) */
const EDGE = 1568;
/** base64 길이. 백엔드 한도(5,000,000)보다 조금 작게 */
const MAX_B64 = 4_800_000;
const TYPES = ["image/png", "image/jpeg", "image/gif", "image/webp"];

let pending: Pending[] = [];
let onChange = () => {};

const b64 = (url: string) => url.slice(url.indexOf(",") + 1);

function load(url: string): Promise<HTMLImageElement> {
  return new Promise((ok, fail) => {
    const img = new Image();
    img.onload = () => ok(img);
    img.onerror = () => fail(new Error("이미지를 읽지 못했습니다"));
    img.src = url;
  });
}

function readUrl(file: Blob): Promise<string> {
  return new Promise((ok, fail) => {
    const r = new FileReader();
    r.onload = () => ok(String(r.result));
    r.onerror = () => fail(r.error);
    r.readAsDataURL(file);
  });
}

/** 크거나 무거우면 긴 변을 EDGE로 줄이고, 그래도 무거우면 JPEG로 */
async function prepare(file: File): Promise<Pending> {
  const url = await readUrl(file);
  const img = await load(url);
  const long = Math.max(img.naturalWidth, img.naturalHeight);
  if (long <= EDGE && b64(url).length <= MAX_B64) return { mediaType: file.type, data: b64(url), url };
  if (file.type === "image/gif") throw new Error("GIF가 너무 큽니다");
  const scale = Math.min(1, EDGE / long);
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(img.naturalWidth * scale);
  canvas.height = Math.round(img.naturalHeight * scale);
  canvas.getContext("2d")!.drawImage(img, 0, 0, canvas.width, canvas.height);
  let type = file.type === "image/jpeg" ? "image/jpeg" : "image/png";
  let out = canvas.toDataURL(type, 0.9);
  if (b64(out).length > MAX_B64) {
    type = "image/jpeg";
    out = canvas.toDataURL(type, 0.85);
  }
  if (b64(out).length > MAX_B64) throw new Error("이미지가 너무 큽니다");
  return { mediaType: type, data: b64(out), url: out };
}

export async function add(files: Iterable<File>) {
  for (const f of files) {
    if (!TYPES.includes(f.type)) {
      toast.warn("PNG, JPEG, GIF, WebP 이미지만 붙일 수 있습니다", f.name);
      continue;
    }
    if (pending.length >= MAX) {
      toast.warn(`이미지는 한 번에 ${MAX}장까지 보낼 수 있습니다`);
      break;
    }
    try {
      pending.push(await prepare(f));
    } catch (e) {
      toast.error("이미지를 붙이지 못했습니다", e instanceof Error ? e.message : String(e));
    }
  }
  render();
}

function render() {
  const box = $("#attachments");
  box.classList.toggle("hidden", !pending.length);
  box.replaceChildren(...pending.map((p, i) => {
    const remove = h("button", { class: "attach-remove", title: "빼기", "aria-label": "이미지 빼기" }, codicon("close"));
    remove.addEventListener("click", () => {
      pending.splice(i, 1);
      render();
    });
    return h("div", { class: "attach-thumb" }, h("img", { src: p.url, alt: "붙인 이미지" }), remove);
  }));
  onChange();
}

export const count = () => pending.length;

/** 보낼 이미지를 가져가고 비운다 */
export function take(): (Image & { url: string })[] {
  const out = pending;
  pending = [];
  render();
  return out;
}

/** 대화에 보이는 붙인 이미지 */
export function thumbs(images: { url: string }[]): HTMLElement {
  return h("div", { class: "user-images" }, ...images.map((p) => h("img", { src: p.url, alt: "붙인 이미지" })));
}

const imagesOf = (items: DataTransfer | null) => [...(items?.files ?? [])].filter((f) => f.type.startsWith("image/"));

export function init(changed: () => void) {
  onChange = changed;
  const prompt = $<HTMLTextAreaElement>("#prompt");
  prompt.addEventListener("paste", (e) => {
    const files = imagesOf(e.clipboardData);
    if (!files.length) return;
    e.preventDefault();
    void add(files);
  });
  const box = $(".composer-box");
  box.addEventListener("dragover", (e) => {
    if (![...(e.dataTransfer?.items ?? [])].some((i) => i.kind === "file")) return;
    e.preventDefault();
    box.classList.add("drop");
  });
  box.addEventListener("dragleave", () => box.classList.remove("drop"));
  box.addEventListener("drop", (e) => {
    box.classList.remove("drop");
    const files = imagesOf(e.dataTransfer);
    if (!files.length) return;
    e.preventDefault();
    void add(files);
  });
  const input = h("input", { type: "file", accept: TYPES.join(","), multiple: true, class: "hidden" });
  input.addEventListener("change", () => {
    void add(input.files ?? []);
    input.value = "";
  });
  const btn = $("#btn-attach");
  btn.after(input);
  btn.addEventListener("click", () => input.click());
}
