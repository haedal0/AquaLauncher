// 전역 UI 상태 — Svelte 5 룬 기반.
import type { UpdateSummary } from "./api";
import { normalizeError, type UiError } from "./errors";
import type { InstanceView, ModItem } from "./types";

export type DialogKind = "add" | "browse" | "progress" | "required" | "update" | "error" | null;

export const ui = $state({
  instances: [] as InstanceView[],
  currentId: "",
  search: "",
  sortRecent: true,
  light: false,
  tab: "home" as "home" | "mods" | "settings",
  dialog: null as DialogKind,
  pendingMod: null as ModItem | null,
  progress: { title: "", stage: "", pct: 0, file: "" },
  error: null as UiError | null,
  /** 인스턴스별 대기 중 업데이트 요약 (§8.2.1 배지 / §8.2.2 확인 다이얼로그) */
  updateSummaries: {} as Record<string, UpdateSummary>,
});

/** §9: 모든 백엔드 에러는 코드+문구+복구 액션 다이얼로그로 표면화한다. */
export function showError(e: unknown, retry?: () => void) {
  ui.error = { ...normalizeError(e), retry };
  ui.dialog = "error";
}

export function current(): InstanceView | undefined {
  return ui.instances.find((i) => i.id === ui.currentId);
}

export function visibleInstances(): InstanceView[] {
  const q = ui.search.toLowerCase();
  const shown = ui.instances.filter((i) => i.name.toLowerCase().includes(q));
  return shown.sort(
    ui.sortRecent ? (a, b) => a.order - b.order : (a, b) => a.name.localeCompare(b.name, "ko"),
  );
}

/// required 모드 비활성 여부로 dirty 상태 재계산 (PRD 8.2.8)
export function recomputeDirty(inst: InstanceView) {
  if (inst.manual) return;
  const reqOff = inst.mods.some((g) => g.items.some((m) => m.kind === "req" && !m.enabled));
  if (inst.state !== "update" && inst.state !== "offline") {
    inst.state = reqOff ? "dirty" : "ok";
  }
}

export function resetToManifest(inst: InstanceView) {
  for (const g of inst.mods) {
    for (const m of g.items) {
      if (m.kind === "req") m.enabled = true;
    }
  }
  recomputeDirty(inst);
}
