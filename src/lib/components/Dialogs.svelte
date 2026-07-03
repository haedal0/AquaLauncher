<script lang="ts">
  // 다이얼로그 6종 — 인스턴스 추가(§8.7/§8.8), 필수 모드 경고(§8.2.8),
  // 대용량 업데이트 확인(§8.2.2), 모드 브라우저(§8.17), 진행(§8.10), 에러(§9).
  import { t } from "../i18n";
  import { detailText, errorMessageKey, isRetryable } from "../errors";
  import {
    LOADERS,
    MC_VERSIONS,
    SWATCH_COLORS,
    createFromManifest,
    createManualInstance,
    hasTauri,
    onDeepLinkAdd,
    previewManifest,
    searchMods,
    type ManifestPreview,
  } from "../api";
  import { startProgress, stopProgress } from "../progress";
  import { current, recomputeDirty, showError, ui } from "../state.svelte";
  import type { BrowseHit, InstanceView } from "../types";

  // ── 인스턴스 추가 ──
  let addManual = $state(false);
  let addStep = $state(1);
  let manifestUrl = $state("");
  let mName = $state("");
  let mVer = $state("1.20.4");
  let mLoader = $state("Fabric");
  let mSnapshots = $state(false);
  let mColor = $state(SWATCH_COLORS[3]);

  let preview = $state<ManifestPreview | null>(null);
  /** 딥링크 중복 감지 (§8.7): 동일 manifest_source 인스턴스 → 기존 열기/새로 만들기 선택 */
  let dupInstance = $state<InstanceView | null>(null);

  function close() {
    if (ui.dialog === "progress") stopProgress();
    ui.dialog = null;
    ui.error = null;
    addStep = 1;
    addManual = false;
    preview = null;
    dupInstance = null;
  }

  // 딥링크 진입 (§8.7): URL 단계는 건너뛰고 확인 모달(미리보기)로 직행
  onDeepLinkAdd(async (url) => {
    addManual = false;
    manifestUrl = url;
    preview = null;
    dupInstance = ui.instances.find((i) => i.manifestSource === url) ?? null;
    ui.dialog = "add";
    if (dupInstance) {
      addStep = 0;
    } else {
      await manifestNext();
    }
  });

  function openExisting() {
    if (dupInstance) ui.currentId = dupInstance.id;
    close();
  }

  async function createAnother() {
    dupInstance = null;
    await manifestNext();
  }

  async function manifestNext() {
    // §8.7 확인 모달: 서버 이름 + 출처 도메인 + 모드 수 + 총 용량
    if (hasTauri) {
      try {
        preview = await previewManifest(manifestUrl.trim());
      } catch (e) {
        // 재시도는 추가 다이얼로그로 복귀 후 재조회 (§9 E-MF-01 등)
        showError(e, () => {
          ui.dialog = "add";
          void manifestNext();
        });
        return;
      }
    }
    addStep = 2;
  }

  async function manifestCreate() {
    let vm: InstanceView | null;
    try {
      vm = await createFromManifest(manifestUrl.trim());
    } catch (e) {
      showError(e, () => void manifestCreate());
      return;
    }
    if (vm) {
      ui.instances.push(vm);
      ui.currentId = vm.id;
    }
    close();
  }

  function fmtBytes(n: number): string {
    if (n >= 1 << 30) return `${(n / (1 << 30)).toFixed(1)} GB`;
    if (n >= 1 << 20) return `${(n / (1 << 20)).toFixed(1)} MB`;
    return `${n} B`;
  }

  async function createManual() {
    const name = mName.trim() || t("add.defaultName");
    let backendVm: InstanceView | null = null;
    if (hasTauri) {
      try {
        backendVm = await createManualInstance(name, mVer, mLoader, mColor);
      } catch (e) {
        showError(e, () => void createManual());
        return;
      }
    }
    const inst: InstanceView = backendVm ?? {
      id: `manual-${Date.now()}`,
      name,
      initial: name[0],
      manual: true,
      desc: t("add.manualDetail"),
      loaderLabel: `${mLoader} ${mVer}`,
      ver: t("sidebar.manual"),
      clog: t("add.manualMemo"),
      last: "—",
      sync: "—",
      disk: "0 MB",
      color: mColor,
      state: "ok",
      playSize: null,
      domain: null,
      manifestSource: null,
      order: ui.instances.length,
      mods: [{ key: "user", items: [] }],
    };
    ui.instances.push(inst);
    ui.currentId = inst.id;
    mName = "";
    close();
  }

  // ── 필수 모드 경고 ──
  function disablePending() {
    const inst = current();
    if (ui.pendingMod && inst) {
      ui.pendingMod.enabled = false;
      recomputeDirty(inst);
    }
    ui.pendingMod = null;
    close();
  }

  // ── 대용량 업데이트 확인 → 동기화 진행 ──
  function applyUpdate() {
    const inst = current();
    ui.dialog = null;
    startProgress(
      t("prog.syncing"),
      [t("prog.stage.mods"), t("prog.stage.resourcepack"), t("prog.stage.commit"), t("prog.stage.launch")],
      () => {
        if (inst) {
          inst.state = "ok";
          inst.playSize = null;
        }
      },
    );
  }

  // ── 모드 브라우저 ──
  let browseSrc = $state<"modrinth" | "curseforge">("modrinth");
  let browseQ = $state("");
  let hits = $state<BrowseHit[]>([]);
  $effect(() => {
    if (ui.dialog === "browse") {
      searchMods(browseSrc, browseQ).then((r) => (hits = r));
    }
  });

  function installed(hit: BrowseHit): boolean {
    const inst = current();
    return !!hit.file && !!inst && inst.mods.some((g) => g.items.some((i) => i.file === hit.file));
  }

  function install(hit: BrowseHit) {
    const inst = current();
    if (!inst || !hit.file) return;
    let group = inst.mods.find((g) => g.key === "user");
    if (!group) {
      group = { key: "user", items: [] };
      inst.mods.push(group);
    }
    // 브라우저 설치 모드는 origin=user — 매니페스트 동기화의 삭제 대상 아님 (§8.17)
    group.items.push({ name: hit.name, file: hit.file, kind: "user", enabled: true });
    if (hit.dep && !inst.mods.some((g) => g.items.some((i) => i.name === hit.dep))) {
      group.items.push({ name: hit.dep, file: `${hit.dep.toLowerCase()}-dep.jar`, kind: "user", enabled: true });
    }
  }

  // ── 에러 (§9: 코드 + 사용자 문구 + 복구 액션) ──
  let errCopied = $state(false);
  $effect(() => {
    if (ui.dialog !== "error") errCopied = false;
  });

  function errRetry() {
    const retry = ui.error?.retry;
    close();
    retry?.();
  }

  async function errCopy() {
    if (!ui.error) return;
    const detail = detailText(ui.error);
    await navigator.clipboard.writeText(detail ? `${ui.error.code}\n${detail}` : ui.error.code);
    errCopied = true;
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && close()} />

{#if ui.dialog === "add"}
  <div class="ovl">
    <div class="dlg" role="dialog" aria-modal="true" aria-label={t("add.title")}>
      <h2>{t("add.title")}</h2>
      <div class="add-modes" role="tablist">
        <button class="add-mode" class:on={!addManual} role="tab" onclick={() => (addManual = false)}>{t("add.modeManifest")}</button>
        <button class="add-mode" class:on={addManual} role="tab" onclick={() => (addManual = true)}>{t("add.modeManual")}</button>
      </div>

      {#if !addManual}
        {#if addStep === 0}
          <p class="dsc">{t("add.dupDesc", { name: dupInstance?.name ?? "" })}</p>
          <span class="src-domain"><i></i>{t("add.trusted", { domain: dupInstance?.domain ?? manifestUrl })}</span>
          <div class="dlg-btns">
            <button class="btn ghost" onclick={openExisting}>{t("add.dupOpen")}</button>
            <button class="btn th" onclick={createAnother}>{t("add.dupNew")}</button>
          </div>
        {:else if addStep === 1}
          <p class="dsc">{t("add.manifestDesc")}</p>
          <div class="field">
            <label for="manifest-url">{t("add.urlLabel")}</label>
            <input id="manifest-url" class="url" bind:value={manifestUrl} />
          </div>
          <div class="dlg-btns">
            <button class="btn ghost" onclick={close}>{t("dlg.cancel")}</button>
            <button class="btn th" onclick={manifestNext}>{t("add.next")}</button>
          </div>
        {:else}
          {#if preview}
            <p class="dsc preview-name"><b>{preview.name}</b><br />{preview.desc ?? ""}</p>
          {/if}
          <span class="src-domain"><i></i>{t("add.trusted", { domain: preview?.domain ?? manifestUrl ?? "…" })}</span>
          <div class="diff" style="margin-top:14px">
            <div><b>{preview?.modCount ?? "—"}</b><span>{t("add.stat.mods")}</span></div>
            <div><b>{preview?.optionalCount ?? "—"}</b><span>{t("add.stat.optional")}</span></div>
            <div><b>{preview ? fmtBytes(preview.totalBytes) : "—"}</b><span>{t("add.stat.download")}</span></div>
          </div>
          <div class="dlg-btns">
            <button class="btn ghost" onclick={close}>{t("dlg.cancel")}</button>
            <button class="btn th" onclick={manifestCreate}>{t("add.create")}</button>
          </div>
        {/if}
      {:else}
        <p class="dsc">{t("add.manualDesc")}</p>
        <div class="field">
          <label for="m-name">{t("add.nameLabel")}</label>
          <input id="m-name" class="url" placeholder={t("add.namePh")} bind:value={mName} />
        </div>
        <div class="field-row">
          <div class="field">
            <label for="m-ver">{t("add.mcVersion")}</label>
            <select id="m-ver" class="url" bind:value={mVer}>
              {#each MC_VERSIONS as v (v)}<option value={v}>{v}</option>{/each}
            </select>
          </div>
          <div class="field">
            <label for="m-loader">{t("add.loader")}</label>
            <select id="m-loader" class="url" bind:value={mLoader}>
              {#each LOADERS as l (l)}<option value={l}>{l}</option>{/each}
            </select>
          </div>
        </div>
        <div class="snap-row">
          <span
            class="tgl" role="switch" tabindex="0" aria-checked={mSnapshots}
            onclick={() => (mSnapshots = !mSnapshots)}
            onkeydown={(e) => { if (e.key === " " || e.key === "Enter") { e.preventDefault(); mSnapshots = !mSnapshots; } }}
          ></span>
          {t("add.snapshots")}
        </div>
        <div class="field">
          <span class="swatch-label">{t("add.iconColor")}</span>
          <div class="swatches">
            {#each SWATCH_COLORS as c (c)}
              <button
                class="swatch" class:on={c === mColor} style="background:{c}"
                aria-label={t("add.colorAria", { color: c })}
                onclick={() => (mColor = c)}
              ></button>
            {/each}
          </div>
        </div>
        <div class="dlg-btns">
          <button class="btn ghost" onclick={close}>{t("dlg.cancel")}</button>
          <button class="btn th" onclick={createManual}>{t("add.createManual")}</button>
        </div>
      {/if}
    </div>
  </div>
{/if}

{#if ui.dialog === "required" && ui.pendingMod}
  <div class="ovl">
    <div class="dlg" role="dialog" aria-modal="true" aria-label={t("req.title")}>
      <h2>{t("req.title")}</h2>
      <p class="dsc">{t("req.desc", { name: ui.pendingMod.name })}</p>
      <div class="dlg-btns">
        <button class="btn ghost" onclick={() => { ui.pendingMod = null; close(); }}>{t("dlg.cancel")}</button>
        <button class="btn warn" onclick={disablePending}>{t("req.go")}</button>
      </div>
    </div>
  </div>
{/if}

{#if ui.dialog === "update"}
  <div class="ovl">
    <div class="dlg" role="dialog" aria-modal="true" aria-label={t("update.title", { name: current()?.name ?? "" })}>
      <h2>{t("update.title", { name: current()?.name ?? "" })}</h2>
      <p class="dsc">{t("update.desc")}</p>
      <!-- TODO(M4): plan_diff 요약(추가/갱신/삭제/총 용량)으로 채움 -->
      <div class="diff">
        <div><b class="ok-tx">+7</b><span>{t("update.added")}</span></div>
        <div><b class="warn-tx">3</b><span>{t("update.changed")}</span></div>
        <div><b class="danger-tx">−2</b><span>{t("update.removed")}</span></div>
        <div><b>{current()?.playSize ?? "—"}</b><span>{t("update.total")}</span></div>
      </div>
      <div class="clog">{current()?.clog}</div>
      <div class="dlg-btns">
        <button class="btn ghost" onclick={close}>{t("dlg.later")}</button>
        <button class="btn th" onclick={applyUpdate}>{t("update.go")}</button>
      </div>
    </div>
  </div>
{/if}

{#if ui.dialog === "browse"}
  <div class="ovl">
    <div class="dlg wide" role="dialog" aria-modal="true" aria-label={t("browse.title")}>
      <h2>{t("browse.title")}</h2>
      <p class="dsc">{t("browse.desc", { compat: current()?.loaderLabel ?? "" })}</p>
      <div class="add-modes" role="tablist">
        <button class="add-mode" class:on={browseSrc === "modrinth"} role="tab" onclick={() => (browseSrc = "modrinth")}>Modrinth</button>
        <button class="add-mode" class:on={browseSrc === "curseforge"} role="tab" onclick={() => (browseSrc = "curseforge")}>CurseForge</button>
      </div>
      <div class="field">
        <input class="url" placeholder={t("browse.searchPh")} aria-label={t("browse.searchPh")} bind:value={browseQ} />
      </div>
      <div class="browse-list">
        {#each hits as hit (browseSrc + hit.name)}
          <div class="b-row">
            <span class="b-ic" style="background:{hit.color}">{hit.initial}</span>
            <span class="b-meta">
              <span class="b-name">{hit.name}</span>
              <div class="b-sub">{hit.sub}</div>
              {#if hit.dep}<div class="b-dep">↳ {t("browse.dep", { name: hit.dep })}</div>{/if}
            </span>
            {#if hit.optOut}
              <!-- CF 배포 opt-out: 숨기지 않고 제한 표시 + 외부 링크 (E-MB-01) -->
              <span class="b-side">
                <button class="b-btn ext">{t("browse.external")}</button>
                <div class="b-lock">{t("browse.optOut")}</div>
              </span>
            {:else if installed(hit)}
              <button class="b-btn done">{t("browse.installed")}</button>
            {:else}
              <button class="b-btn" onclick={() => install(hit)}>{t("browse.install")}</button>
            {/if}
          </div>
        {:else}
          <div class="empty-box"><b>{t("browse.emptyTitle")}</b>{t("browse.emptyDesc")}</div>
        {/each}
      </div>
      <div class="dlg-btns"><button class="btn ghost" onclick={close}>{t("dlg.close")}</button></div>
    </div>
  </div>
{/if}

{#if ui.dialog === "error" && ui.error}
  <div class="ovl">
    <div class="dlg" role="dialog" aria-modal="true" aria-label={t("err.title")}>
      <h2>{t("err.title")}</h2>
      <p class="dsc">{t(errorMessageKey(ui.error.code))}</p>
      <span class="err-code">{ui.error.code}</span>
      {#if detailText(ui.error)}
        <div class="clog err-detail">{detailText(ui.error)}</div>
      {/if}
      <div class="dlg-btns">
        <button class="btn ghost" onclick={errCopy}>{errCopied ? t("err.copied") : t("err.copy")}</button>
        <button class="btn ghost" onclick={close}>{t("dlg.close")}</button>
        {#if ui.error.retry && isRetryable(ui.error.code)}
          <button class="btn th" onclick={errRetry}>{t("err.retry")}</button>
        {/if}
      </div>
    </div>
  </div>
{/if}

{#if ui.dialog === "progress"}
  <div class="ovl">
    <div class="dlg" role="dialog" aria-modal="true" aria-label={ui.progress.title}>
      <h2>{ui.progress.title}</h2>
      <div class="prog-wrap">
        <div class="prog-stage"><b>{ui.progress.stage}</b><span>{Math.floor(ui.progress.pct)}%</span></div>
        <div class="prog-bar"><i style="width:{ui.progress.pct}%"></i></div>
        <div class="prog-file">{ui.progress.file}</div>
      </div>
      <div class="dlg-btns">
        <button class="btn ghost" onclick={close}>{t("prog.cancel")}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .ovl { position: fixed; inset: 0; background: rgba(8, 9, 12, 0.55); backdrop-filter: blur(3px);
    display: grid; place-items: center; z-index: 20; }
  .dlg { width: min(470px, 92%); background: var(--chrome-panel); border: 1px solid var(--chrome-line);
    border-radius: 14px; padding: 22px; box-shadow: 0 24px 60px rgba(0, 0, 0, 0.4); }
  .dlg.wide { width: min(540px, 94%); }
  .dlg h2 { font-size: 1.05rem; font-weight: 800; letter-spacing: -0.01em; }
  .dsc { font-size: 0.84rem; color: var(--tx-dim); margin-top: 7px; line-height: 1.55; }
  .dlg-btns { display: flex; gap: 8px; margin-top: 18px; justify-content: flex-end; }
  .clog { font-size: 0.8rem; color: var(--tx-dim); background: var(--chrome-bg);
    border-radius: 9px; padding: 10px 12px; line-height: 1.5; border: 1px solid var(--chrome-line); }
  .ok-tx { color: var(--ok); }
  .warn-tx { color: var(--warn); }
  .danger-tx { color: var(--danger); }
  .src-domain { display: inline-flex; align-items: center; gap: 5px; font-size: 0.72rem; font-weight: 700;
    color: var(--tx-dim); background: var(--chrome-bg); border: 1px solid var(--chrome-line);
    padding: 3px 9px; border-radius: 99px; margin-top: 10px; }
  .src-domain i { width: 7px; height: 7px; border-radius: 99px; background: var(--ok); flex: none; }
  .field-row { display: flex; gap: 10px; }
  .field-row .field { flex: 1; }
  .snap-row { display: flex; align-items: center; gap: 8px; margin-top: 10px;
    font-size: 0.76rem; color: var(--tx-faint); }
  .snap-row .tgl { margin-left: 0; scale: 0.85; }
  .swatch-label { display: block; font-size: 0.72rem; font-weight: 700; color: var(--tx-faint);
    letter-spacing: 0.06em; margin-bottom: 5px; }
  .swatches { display: flex; gap: 8px; margin-top: 4px; }
  .swatch { width: 30px; height: 30px; border-radius: 9px; border: 2px solid transparent;
    position: relative; transition: 0.12s; }
  .swatch.on { border-color: var(--tx); scale: 1.08; }
  .browse-list { max-height: 270px; overflow-y: auto; margin-top: 12px; padding-right: 2px; }
  .browse-list::-webkit-scrollbar { width: 7px; }
  .browse-list::-webkit-scrollbar-thumb { background: var(--chrome-raise); border-radius: 99px; }
  .b-row { display: flex; align-items: center; gap: 11px; background: var(--chrome-bg);
    border: 1px solid var(--chrome-line); border-radius: 9px; padding: 9px 12px; margin-bottom: 6px; }
  .b-ic { width: 32px; height: 32px; border-radius: 8px; flex: none; display: grid; place-items: center;
    font-size: 14px; font-weight: 800; color: #fff; }
  .b-meta { min-width: 0; flex: 1; }
  .b-name { font-size: 0.85rem; font-weight: 700; }
  .b-sub { font-size: 0.7rem; color: var(--tx-faint); margin-top: 1px; }
  .b-dep { font-size: 0.66rem; color: var(--th-tx); margin-top: 2px; }
  .b-side { flex: none; text-align: right; }
  .b-btn { flex: none; font-size: 0.75rem; font-weight: 700; background: var(--th); color: #fff;
    border-radius: 7px; padding: 6px 13px; }
  .b-btn:hover { filter: brightness(1.08); }
  .b-btn.done { background: var(--chrome-raise); color: var(--tx-dim); cursor: default; }
  .b-btn.ext { background: none; border: 1px solid var(--chrome-line); color: var(--tx-dim); font-weight: 600; }
  .b-btn.ext:hover { color: var(--tx); }
  .b-lock { font-size: 0.64rem; color: var(--warn); margin-top: 2px; }
  .prog-wrap { margin-top: 16px; }
  .prog-stage { display: flex; justify-content: space-between; font-size: 0.78rem;
    color: var(--tx-dim); margin-bottom: 7px; }
  .prog-stage b { color: var(--tx); }
  .prog-bar { height: 8px; border-radius: 99px; background: var(--chrome-bg); overflow: hidden;
    border: 1px solid var(--chrome-line); }
  .prog-bar i { display: block; height: 100%; background: var(--th); border-radius: 99px;
    transition: width 0.18s; }
  .prog-file { font-size: 0.7rem; color: var(--tx-faint); margin-top: 7px;
    font-variant-numeric: tabular-nums; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .err-code { display: inline-block; font-size: 0.68rem; font-weight: 700; letter-spacing: 0.05em;
    color: var(--danger); background: rgba(229, 97, 91, 0.1); border: 1px solid rgba(229, 97, 91, 0.35);
    border-radius: 99px; padding: 2px 9px; margin-top: 10px; }
  .err-detail { margin-top: 10px; max-height: 130px; overflow-y: auto;
    font-family: ui-monospace, monospace; font-size: 0.72rem; white-space: pre-wrap; word-break: break-all; }
</style>
