<script lang="ts">
  // 사이드바 — PRD 8.14: 검색 + 정렬 + 상태 배지 + 하단 추가 버튼 고정.
  import { t } from "../i18n";
  import { softOf } from "../theme";
  import { ui, visibleInstances } from "../state.svelte";

  const badgeLabel: Record<string, string> = {
    update: t("badge.update"),
    dirty: t("badge.dirty"),
    offline: t("badge.offline"),
  };
</script>

<nav class="side" aria-label={t("sidebar.myServers")}>
  <div class="side-head">
    <div class="brand">
      <div class="brand-cube" aria-hidden="true"></div>
      <b>AquaLauncher</b>
      <span class="ver-pill">v0.1</span>
      <button
        class="theme-btn"
        title={t("sidebar.themeToggle")}
        aria-label={t("sidebar.themeToggle")}
        onclick={() => (ui.light = !ui.light)}
      >{ui.light ? "☾" : "☀"}</button>
    </div>
    <div class="search">
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>
      <input type="search" placeholder={t("sidebar.search")} aria-label={t("sidebar.search")} bind:value={ui.search} />
    </div>
  </div>
  <div class="side-label">
    <span>{t("sidebar.myServers")} · {visibleInstances().length}</span>
    <button class="sort-btn" onclick={() => (ui.sortRecent = !ui.sortRecent)}>
      {ui.sortRecent ? t("sidebar.sortRecent") : t("sidebar.sortName")}
    </button>
  </div>
  <div class="inst-list" role="listbox" aria-label={t("sidebar.myServers")}>
    {#each visibleInstances() as inst (inst.id)}
      <button
        class="inst"
        class:active={inst.id === ui.currentId}
        role="option"
        aria-selected={inst.id === ui.currentId}
        style="--ic:{inst.color};--ic-bg:{inst.color};--ic-soft:{softOf(inst.color)}"
        onclick={() => (ui.currentId = inst.id)}
      >
        <span class="inst-icon">{inst.initial}</span>
        <span class="inst-meta">
          <span class="inst-name">{inst.name}</span>
          <span class="inst-sub">{inst.loaderLabel}{inst.manual ? ` · ${t("sidebar.manual")}` : ""}</span>
        </span>
        {#if inst.state !== "ok"}
          <span class="badge {inst.state}">{badgeLabel[inst.state]}</span>
        {/if}
      </button>
    {/each}
  </div>
  <div class="side-foot">
    <button class="add-btn" onclick={() => (ui.dialog = "add")}>
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6"><path d="M12 5v14M5 12h14"/></svg>
      {t("sidebar.add")}
    </button>
  </div>
</nav>

<style>
  .side { background: var(--chrome-panel); border-right: 1px solid var(--chrome-line);
    display: flex; flex-direction: column; min-height: 0; }
  .side-head { padding: 16px 14px 10px; }
  .brand { display: flex; align-items: center; gap: 9px; padding: 2px 4px 12px; }
  .brand-cube { width: 22px; height: 22px; border-radius: 6px; flex: none;
    background: conic-gradient(from 45deg, #6f7687 25%, #4a4f5e 0 50%, #8b93a6 0 75%, #5a6072 0); }
  .brand b { font-size: 0.95rem; letter-spacing: -0.01em; }
  .ver-pill { font-size: 0.7rem; color: var(--tx-faint); margin-left: auto;
    border: 1px solid var(--chrome-line); border-radius: 99px; padding: 2px 7px; }
  .theme-btn { width: 26px; height: 26px; border-radius: 7px; display: grid; place-items: center;
    color: var(--tx-dim); border: 1px solid var(--chrome-line); flex: none; font-size: 0.8rem; }
  .theme-btn:hover { color: var(--tx); background: var(--chrome-raise); }
  .search { position: relative; }
  .search input { width: 100%; background: var(--chrome-bg); border: 1px solid var(--chrome-line);
    border-radius: 8px; padding: 8px 10px 8px 30px; color: var(--tx); font-size: 0.85rem; }
  .search input::placeholder { color: var(--tx-faint); }
  .search svg { position: absolute; left: 9px; top: 50%; translate: 0 -50%; opacity: 0.45; }
  .side-label { display: flex; align-items: center; justify-content: space-between;
    padding: 12px 18px 6px; font-size: 0.68rem; letter-spacing: 0.12em; color: var(--tx-faint); font-weight: 600; }
  .sort-btn { font-size: 0.68rem; color: var(--tx-faint); display: flex; gap: 4px; align-items: center;
    padding: 2px 4px; border-radius: 5px; }
  .sort-btn:hover { color: var(--tx-dim); }
  .inst-list { flex: 1; overflow-y: auto; padding: 2px 8px 8px;
    display: flex; flex-direction: column; gap: 2px; min-height: 0; }
  .inst-list::-webkit-scrollbar { width: 8px; }
  .inst-list::-webkit-scrollbar-thumb { background: var(--chrome-raise); border-radius: 99px; }
  .inst { display: flex; align-items: center; gap: 10px; width: 100%;
    padding: 8px 10px; border-radius: 9px; text-align: left;
    border: 1px solid transparent; transition: background 0.12s; }
  .inst:hover { background: var(--chrome-raise); }
  .inst.active { background: var(--chrome-raise); border-color: var(--chrome-line);
    box-shadow: inset 3px 0 0 var(--ic, #888); }
  .inst-icon { width: 34px; height: 34px; border-radius: 8px; flex: none;
    display: grid; place-items: center; font-size: 15px;
    background: var(--ic-bg, #333); color: #fff; font-weight: 700; position: relative; overflow: hidden; }
  .inst-icon::after { content: ""; position: absolute; inset: 0;
    background: linear-gradient(135deg, rgba(255, 255, 255, 0.22), transparent 55%); }
  .inst-meta { min-width: 0; flex: 1; }
  .inst-name { display: block; font-size: 0.86rem; font-weight: 600;
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .inst-sub { display: block; font-size: 0.7rem; color: var(--tx-faint);
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis; margin-top: 1px; }
  .badge { flex: none; font-size: 0.62rem; font-weight: 700; padding: 2.5px 6px;
    border-radius: 99px; letter-spacing: 0.02em; }
  .badge.update { background: var(--ic-soft, rgba(120, 160, 255, 0.18)); color: var(--ic, #9db8ff); }
  .badge.dirty { background: rgba(224, 179, 76, 0.15); color: var(--warn); }
  .badge.offline { background: rgba(229, 97, 91, 0.13); color: var(--danger); }
  .side-foot { padding: 10px; border-top: 1px solid var(--chrome-line); }
  .add-btn { width: 100%; display: flex; align-items: center; justify-content: center; gap: 7px;
    border: 1px dashed var(--chrome-line); border-radius: 9px; padding: 9px;
    color: var(--tx-dim); font-size: 0.85rem; font-weight: 600; transition: 0.12s; }
  .add-btn:hover { color: var(--tx); border-color: var(--tx-faint); border-style: solid; }
</style>
