<script lang="ts">
  // 인스턴스 상세 — PRD 8.14: 테마 히어로 + 경고 스트립 + 탭 + 고정 플레이 도크.
  import { t } from "../i18n";
  import { ACCOUNT_NAME, hasTauri, playBackend, resetBackend } from "../api";
  import { startProgress } from "../progress";
  import type { InstanceView } from "../types";
  import { resetToManifest, ui } from "../state.svelte";
  import ModsPanel from "./ModsPanel.svelte";

  let { inst }: { inst: InstanceView } = $props();

  const tabs = ["home", "mods", "settings"] as const;

  let modCount = $derived(inst.mods.reduce((n, g) => n + g.items.length, 0));

  function dockInfo(): string {
    if (inst.state === "update") return t("dock.updatePending", { size: inst.playSize ?? "?" });
    if (inst.state === "offline") return t("dock.offline");
    if (inst.manual) return t("dock.readyManual", { account: ACCOUNT_NAME });
    return t("dock.ready", { account: ACCOUNT_NAME });
  }

  function play() {
    if (inst.state === "update") {
      ui.dialog = "update";
      return;
    }
    if (hasTauri) {
      // 실 파이프라인: 진행 이벤트(App에서 구독)가 다이얼로그를 채운다
      ui.progress = { title: t("prog.preparing"), stage: t("prog.stage.sync"), pct: 0, file: "" };
      ui.dialog = "progress";
      playBackend(inst.id)
        .then(() => {
          if (ui.dialog === "progress") ui.dialog = null;
        })
        .catch((e) => {
          console.error(e);
          if (ui.dialog === "progress") ui.dialog = null;
        });
      return;
    }
    startProgress(t("prog.preparing"), [t("prog.stage.verify"), t("prog.stage.classpath"), t("prog.stage.launch")]);
  }

  function resetAll() {
    resetToManifest(inst);
    void resetBackend(inst.id);
  }
</script>

<main class="main">
  <header class="hero">
    <div class="hero-top">
      <div class="hero-logo">{inst.initial}</div>
      <div class="hero-title">
        <h1>{inst.name}</h1>
        <p>{inst.desc}</p>
        <div class="hero-tags">
          <span class="tag th">{inst.loaderLabel}</span>
          <span class="tag">{inst.domain ?? t("tag.manual")}</span>
          <span class="tag">{inst.ver}</span>
        </div>
      </div>
      <div class="hero-acct"><span class="av"></span><span>{ACCOUNT_NAME} ▾</span></div>
    </div>
  </header>

  {#if inst.state === "dirty"}
    <div class="strip warn">
      {t("strip.dirty")}
      <button onclick={resetAll}>{t("strip.reset")}</button>
    </div>
  {/if}
  {#if inst.state === "offline"}
    <div class="strip err">
      {t("strip.offline", { time: inst.sync })}
      <button>{t("strip.retry")}</button>
    </div>
  {/if}

  <div class="tabs" role="tablist">
    {#each tabs as tab (tab)}
      <button
        class="tab"
        class:active={ui.tab === tab}
        role="tab"
        aria-selected={ui.tab === tab}
        onclick={() => (ui.tab = tab)}
      >
        {t(`tab.${tab}`)}{tab === "mods" && modCount ? ` ${modCount}` : ""}
      </button>
    {/each}
  </div>

  {#if ui.tab === "home"}
    <section class="panel" role="tabpanel">
      <div class="cards">
        <div class="card changelog">
          <h3>{inst.manual ? t("home.memo") : t("home.changelog")}</h3>
          <p><span class="ver">{inst.ver}</span>{inst.clog}</p>
        </div>
        <div class="card">
          <h3>{t("home.lastPlayed")}</h3>
          <div class="big">{inst.last}</div>
          <div class="sub">{inst.sync}</div>
        </div>
        <div class="card">
          <h3>{t("home.disk")}</h3>
          <div class="big">{inst.disk}</div>
          <div class="sub">{t("home.diskSub")}</div>
        </div>
      </div>
    </section>
  {:else if ui.tab === "mods"}
    <section class="panel" role="tabpanel">
      <ModsPanel {inst} />
    </section>
  {:else}
    <section class="panel" role="tabpanel">
      {#if !inst.manual}
        <div class="set-row">
          <div><div>{t("settings.direct")}</div><div class="d">{t("settings.directDesc")}</div></div>
          <span class="set-val"><span class="tgl" role="switch" aria-checked="true" tabindex="0"></span></span>
        </div>
        <div class="set-row">
          <div><div>{t("settings.auto")}</div><div class="d">{t("settings.autoDesc")}</div></div>
          <span class="set-val"><span class="tgl" role="switch" aria-checked="true" tabindex="0"></span></span>
        </div>
      {/if}
      <div class="set-row">
        <div><div>{t("settings.memory")}</div><div class="d">{t("settings.memoryDesc")}</div></div>
        <span class="set-val">2 GB – 4 GB <button class="mini-btn">{t("settings.change")}</button></span>
      </div>
      <div class="set-row">
        <div><div>{t("settings.java")}</div><div class="d">{t("settings.javaDesc")}</div></div>
        <span class="set-val">JRE 17 <button class="mini-btn">{t("settings.javaPick")}</button></span>
      </div>
      <div class="set-row">
        <div><div>{t("settings.verify")}</div><div class="d">{t("settings.verifyDesc")}</div></div>
        <span class="set-val"><button class="mini-btn">{t("settings.verifyRun")}</button></span>
      </div>
      <div class="set-row">
        <div><div>{t("settings.delete")}</div><div class="d">{t("settings.deleteDesc")}</div></div>
        <span class="set-val">
          <button class="mini-btn danger">{t("settings.deleteBtn")}</button>
        </span>
      </div>
    </section>
  {/if}

  <div class="dock">
    <div class="dock-info">{dockInfo()}</div>
    <button class="play" onclick={play}>
      <svg width="17" height="17" viewBox="0 0 24 24" fill="currentColor"><path d="M7 4.5v15l13-7.5z"/></svg>
      <span>
        <span>{inst.state === "update" ? t("dock.playUpdate") : t("dock.play")}</span>
        <span class="pl-sub">{inst.loaderLabel}</span>
      </span>
    </button>
  </div>
</main>

<style>
  .main { position: relative; display: flex; flex-direction: column; min-width: 0;
    background:
      radial-gradient(1200px 500px at 80% -10%, var(--th-soft), transparent 60%),
      linear-gradient(165deg, var(--th-bg2), var(--th-bg1) 70%);
    transition: background 0.45s ease; }
  .main::before { content: ""; position: absolute; inset: 0; pointer-events: none; opacity: 0.6;
    background-image:
      linear-gradient(var(--grid-line) 1px, transparent 1px),
      linear-gradient(90deg, var(--grid-line) 1px, transparent 1px);
    background-size: 44px 44px;
    mask-image: linear-gradient(to bottom, #000 0%, transparent 62%); }
  .hero { padding: 30px 34px 0; position: relative; }
  .hero-top { display: flex; align-items: flex-start; gap: 16px; }
  .hero-logo { width: 62px; height: 62px; border-radius: 14px; flex: none;
    display: grid; place-items: center; font-size: 26px; font-weight: 800; color: #fff;
    background: var(--th); box-shadow: 0 8px 24px -6px var(--th); position: relative; overflow: hidden; }
  .hero-logo::after { content: ""; position: absolute; inset: 0;
    background: linear-gradient(135deg, rgba(255, 255, 255, 0.3), transparent 55%); }
  .hero-title h1 { font-size: 1.55rem; font-weight: 800; letter-spacing: -0.02em; line-height: 1.15; }
  .hero-title p { color: var(--tx-dim); font-size: 0.86rem; margin-top: 5px; max-width: 52ch; }
  .hero-tags { display: flex; gap: 6px; margin-top: 10px; flex-wrap: wrap; }
  .tag { font-size: 0.7rem; font-weight: 600; color: var(--tx-dim);
    background: var(--surf); border: 1px solid var(--surf-line);
    padding: 3px 9px; border-radius: 99px; }
  .tag.th { color: var(--th-tx); background: var(--th-soft); border-color: transparent; }
  .hero-acct { margin-left: auto; flex: none; display: flex; align-items: center; gap: 8px;
    background: var(--surf); border: 1px solid var(--surf-line);
    border-radius: 99px; padding: 5px 12px 5px 6px; font-size: 0.78rem; color: var(--tx-dim); }
  .av { width: 22px; height: 22px; border-radius: 6px;
    background: linear-gradient(135deg, #c98a5b, #8a5a3b); }
  .strip { margin: 16px 34px 0; border-radius: 9px; padding: 9px 13px; font-size: 0.8rem;
    display: flex; align-items: center; gap: 9px; border: 1px solid; position: relative; }
  .strip.warn { background: rgba(224, 179, 76, 0.1); border-color: rgba(224, 179, 76, 0.35); color: var(--warn); }
  .strip.err { background: rgba(229, 97, 91, 0.1); border-color: rgba(229, 97, 91, 0.35); color: var(--danger); }
  .strip button { margin-left: auto; font-size: 0.75rem; font-weight: 700;
    text-decoration: underline; text-underline-offset: 3px; flex: none; }
  .tabs { display: flex; gap: 2px; padding: 20px 34px 0; position: relative; }
  .tab { padding: 8px 14px; font-size: 0.86rem; font-weight: 600; color: var(--tx-dim);
    border-radius: 8px 8px 0 0; border-bottom: 2px solid transparent; }
  .tab:hover { color: var(--tx); }
  .tab.active { color: var(--tx); border-bottom-color: var(--th); }
  .panel { flex: 1; overflow-y: auto; padding: 18px 34px 110px; position: relative; min-height: 0; }
  .panel::-webkit-scrollbar { width: 8px; }
  .panel::-webkit-scrollbar-thumb { background: var(--tgl-off); border-radius: 99px; }
  .cards { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .card { background: var(--surf); border: 1px solid var(--surf-line);
    border-radius: var(--radius); padding: 14px 16px; }
  .card h3 { font-size: 0.72rem; letter-spacing: 0.1em; color: var(--tx-faint);
    font-weight: 700; margin-bottom: 9px; }
  .card .big { font-size: 1.05rem; font-weight: 700; }
  .card .sub { font-size: 0.76rem; color: var(--tx-dim); margin-top: 3px; }
  .changelog { grid-column: 1 / -1; }
  .changelog p { font-size: 0.84rem; color: var(--tx-dim); line-height: 1.55; }
  .changelog .ver { color: var(--th-tx); font-weight: 700; font-size: 0.8rem;
    background: var(--th-soft); border-radius: 6px; padding: 2px 8px; margin-right: 8px; }
  .set-row { display: flex; align-items: center; gap: 12px; padding: 13px 2px;
    border-bottom: 1px solid var(--surf-line); font-size: 0.86rem; }
  .set-row .d { font-size: 0.74rem; color: var(--tx-faint); margin-top: 2px; }
  .set-val { margin-left: auto; color: var(--tx-dim); font-size: 0.8rem; flex: none;
    display: flex; align-items: center; gap: 8px; }
  .mini-btn.danger { color: var(--danger); border-color: rgba(229, 97, 91, 0.4); }
  .dock { position: absolute; left: 0; right: 0; bottom: 0; padding: 16px 34px 20px;
    display: flex; align-items: center; gap: 14px;
    background: linear-gradient(to top, var(--dock-fade) 30%, transparent); }
  .dock-info { font-size: 0.76rem; color: var(--tx-dim); }
  .play { margin-left: auto; flex: none; display: flex; align-items: center; gap: 10px;
    background: var(--th); color: #fff; font-weight: 800; font-size: 1rem; letter-spacing: -0.01em;
    padding: 13px 34px; border-radius: 12px;
    box-shadow: 0 10px 30px -8px var(--th); transition: filter 0.12s, translate 0.12s; }
  .play:hover { filter: brightness(1.1); }
  .play:active { translate: 0 1px; }
  .pl-sub { font-size: 0.68rem; font-weight: 600; opacity: 0.85; display: block; letter-spacing: 0.02em; }
  .play svg { flex: none; }
</style>
