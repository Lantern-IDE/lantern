// 네이티브 대화상자. 화면 언어에 맞춰 글자를 바꾼다 (DOM 밖이라 i18n 관찰자가 못 본다).
import { ask as nativeAsk, message as nativeMessage, open as nativeOpen, type ConfirmDialogOptions, type MessageDialogOptions, type OpenDialogOptions } from "@tauri-apps/plugin-dialog";
import { t } from "./i18n";

function tr<T extends { title?: string; okLabel?: string; cancelLabel?: string }>(o?: T): T | undefined {
  if (!o) return o;
  return { ...o, title: o.title && t(o.title), okLabel: o.okLabel && t(o.okLabel), cancelLabel: o.cancelLabel && t(o.cancelLabel) };
}

export const ask = (text: string, opts?: ConfirmDialogOptions) => nativeAsk(t(text), tr(opts));
export const message = (text: string, opts?: MessageDialogOptions) => nativeMessage(t(text), tr(opts));
export const open = (opts?: OpenDialogOptions) => nativeOpen(opts && { ...opts, title: opts.title && t(opts.title) });
