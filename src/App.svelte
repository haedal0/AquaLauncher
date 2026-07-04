<script lang="ts">
  // 레이아웃 — PRD 8.14: 좌측 사이드바 + 우측 테마 상세. docs/mockup.html 기준.
  import { t } from "./lib/i18n";
  import { checkUpdate, fetchInstances, hasTauri, onGameExited, onProgress } from "./lib/api";
  import { fmtBytes } from "./lib/format";
  import { applyThemeVars } from "./lib/theme";
  import { current, showError, ui } from "./lib/state.svelte";
  import Sidebar from "./lib/components/Sidebar.svelte";
  import Detail from "./lib/components/Detail.svelte";
  import Dialogs from "./lib/components/Dialogs.svelte";

  $effect(() => {
    fetchInstances().then((list) => {
      ui.instances = list;
      if (list.length && !ui.currentId) ui.currentId = list[0].id;
      refreshUpdateBadges();
    });
  });

  // §8.2.1 런처 시작 시: 해시 비교만 백그라운드 수행 → "업데이트 있음 (총 용량)" 배지.
  // 다운로드는 하지 않는다. 오프라인이면 checkUpdate가 null — 배지 없음.
  function refreshUpdateBadges() {
    if (!hasTauri) return;
    for (const { id, manifestSource } of ui.instances) {
      if (!manifestSource) continue;
      checkUpdate(id).then((summary) => {
        if (!summary) return;
        ui.updateSummaries[id] = summary;
        const inst = ui.instances.find((i) => i.id === id);
        if (inst && inst.state === "ok") {
          inst.state = "update";
          inst.playSize = fmtBytes(summary.totalBytes);
        }
      });
    }
  }

  // 백엔드 진행 이벤트(100ms 스로틀) → 진행 다이얼로그 (PRD 8.10)
  const stagePct: Record<string, number> = { sync: 10, loader: 25, download: 60, java: 85, launch: 97 };
  onProgress((p) => {
    if (ui.dialog !== "progress") return;
    ui.progress.stage = t(`prog.stage.${p.stage}`);
    ui.progress.pct = stagePct[p.stage] ?? ui.progress.pct;
    ui.progress.file = p.item;
  });
  onGameExited((p) => {
    fetchInstances().then((list) => (ui.instances = list));
    // §9 E-GM-01/02: 비정상 종료 코드는 에러 다이얼로그로 표면화 (§8.12)
    if (p.code !== null && p.code !== 0) {
      showError({ code: p.crashLoop ? "E-GM-02" : "E-GM-01", detail: `exit code ${p.code}` });
    }
  });

  // 런처 외형 라이트/다크 + 서버 테마 파생 (PRD 8.5 / 8.14)
  $effect(() => {
    document.body.dataset.theme = ui.light ? "light" : "dark";
    const inst = current();
    if (inst) applyThemeVars(inst.color, ui.light);
  });

  // OS 테마 변경 실시간 추종 — 수동 토글은 다음 OS 변경 전까지 유지
  $effect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: light)");
    const onChange = (e: MediaQueryListEvent) => (ui.light = e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  });
</script>

<div class="app">
  <Sidebar />
  {#if current()}
    <Detail inst={current()!} />
  {:else}
    <!-- 인스턴스 0개: 온보딩 안내 (PRD 8.14) -->
    <main class="onboard">
      <div class="empty-box">
        <b>{t("empty.title")}</b>
        {t("empty.desc")}
        <div class="onboard-btn">
          <button class="btn th" onclick={() => (ui.dialog = "add")}>{t("sidebar.add")}</button>
        </div>
      </div>
    </main>
  {/if}
</div>
<Dialogs />

<style>
  .app { display: grid; grid-template-columns: 264px 1fr; height: 100vh; }
  .onboard { display: grid; place-items: center; padding: 24px; }
  .onboard-btn { margin-top: 14px; }
</style>
