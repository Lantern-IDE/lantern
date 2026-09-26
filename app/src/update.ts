// 업데이트 확인. 시작할 때마다, 그리고 켜 둔 동안 몇 시간마다 조용히 확인하고, 메뉴에서 직접 확인할 수도 있다.
// "나중에"로 닫은 버전은 이번 실행 동안 다시 띄우지 않는다 (다음에 켜면 다시 알린다).
// 설치 파일은 서명을 검증한 뒤에만 설치된다 (tauri.conf.json의 plugins.updater.pubkey).
import { invoke } from "@tauri-apps/api/core";
import { errorText } from "./api";
import * as toast from "./toast";

interface UpdateInfo { version: string; notes: string | null }
const HOURS = 60 * 60 * 1000;
/** 켜 둔 동안 다시 확인하는 간격 */
const RECHECK = 6 * HOURS;

/** 이번 실행에서 이미 알린 버전 (알림이 떠 있거나 "나중에"로 닫음) */
const shown = new Set<string>();

async function install(version: string) {
  const close = toast.show({ kind: "info", message: `Lantern ${version}을(를) 내려받는 중…`, detail: "설치가 끝나면 자동으로 다시 시작합니다.", timeout: 0 });
  try {
    await invoke("install_update");
  } catch (e) {
    close();
    toast.error("업데이트를 설치하지 못했습니다", errorText(e));
  }
}

/** manual이면 결과(최신/실패)를 항상 알리고, 이미 알린 버전도 다시 띄운다 */
export async function check(manual: boolean) {
  try {
    const u = await invoke<UpdateInfo | null>("check_update");
    if (!u) {
      if (manual) toast.success("최신 버전을 쓰고 있습니다");
      return;
    }
    if (!manual && shown.has(u.version)) return;
    shown.add(u.version);
    toast.show({
      kind: "info",
      message: `새 버전 Lantern ${u.version}이(가) 있습니다`,
      detail: u.notes?.split("\n").slice(0, 3).join(" ") || undefined,
      timeout: 0,
      actions: [{ label: "설치하고 다시 시작", primary: true, run: () => void install(u.version) }, { label: "나중에", run: () => {} }],
    });
  } catch (e) {
    if (manual) toast.warn("업데이트를 확인하지 못했습니다", errorText(e));
  }
}

/** 시작 직후 한 번, 그 뒤로 RECHECK마다 */
export function schedule(firstDelayMs: number) {
  window.setTimeout(() => void check(false), firstDelayMs);
  window.setInterval(() => void check(false), RECHECK);
}
