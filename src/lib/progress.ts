// 진행 오버레이 구동 — PRD 8.10.
// 브라우저 dev 전용 목 진행. Tauri에서는 백엔드 progress 이벤트(App.svelte 구독)가
// ui.progress를 직접 채우므로 이 타이머는 돌지 않는다.
import { t } from "./i18n";
import { ui } from "./state.svelte";

let timer: ReturnType<typeof setInterval> | null = null;

export function startProgress(title: string, stages: string[], onDone?: () => void) {
  ui.progress = { title, stage: stages[0], pct: 0, file: "" };
  ui.dialog = "progress";
  if (timer) clearInterval(timer);
  timer = setInterval(() => {
    let p = ui.progress.pct + Math.random() * 7 + 3;
    if (p >= 100) {
      p = 100;
      stopProgress();
      onDone?.();
      setTimeout(() => {
        if (ui.dialog === "progress") ui.dialog = null;
      }, 500);
    }
    ui.progress.pct = p;
    ui.progress.stage = stages[Math.min(Math.floor(p / (100 / stages.length)), stages.length - 1)];
    ui.progress.file = p < 100 ? `${(Math.random() * 8 + 2).toFixed(1)} MB/s` : t("prog.done");
  }, 160);
}

// 취소: 스테이징 폐기 = 기존 상태 유지 (PRD 8.2.2)
export function stopProgress() {
  if (timer) clearInterval(timer);
  timer = null;
}
