<script lang="ts">
  // 모드 탭 — PRD 8.2.8 (토글/경고/초기화) + 8.17 진입점.
  import { t } from "../i18n";
  import { resetBackend, toggleModBackend } from "../api";
  import type { InstanceView, ModItem } from "../types";
  import { recomputeDirty, resetToManifest, ui } from "../state.svelte";

  let { inst }: { inst: InstanceView } = $props();

  const groupLabel: Record<string, string> = {
    required: t("mods.groupRequired"),
    optional: t("mods.groupOptional"),
    user: t("mods.groupUser"),
  };

  let total = $derived(inst.mods.reduce((n, g) => n + g.items.length, 0));
  let off = $derived(inst.mods.reduce((n, g) => n + g.items.filter((m) => !m.enabled).length, 0));

  function toggle(m: ModItem) {
    if (m.kind === "req" && m.enabled) {
      // required 모드 비활성화는 경고 다이얼로그를 거친다 (PRD 8.2.8 확정)
      ui.pendingMod = m;
      ui.dialog = "required";
      return;
    }
    m.enabled = !m.enabled;
    recomputeDirty(inst);
    void toggleModBackend(inst.id, `mods/${m.file}`, m.enabled);
  }

  function resetAll() {
    resetToManifest(inst);
    void resetBackend(inst.id);
  }
</script>

<div class="mods-head">
  <span class="cnt">{total ? t("mods.summary", { total, off }) : t("mods.none")}</span>
  <button class="reset-btn" onclick={() => (ui.dialog = "browse")}>{t("mods.browse")}</button>
  {#if !inst.manual}
    <button class="reset-btn" onclick={resetAll}>{t("mods.reset")}</button>
  {/if}
</div>

{#if total === 0}
  <div class="empty-box"><b>{t("mods.emptyTitle")}</b>{t("mods.emptyDesc")}</div>
{:else}
  {#each inst.mods as group (group.key)}
    {#if group.items.length}
      <div class="mod-group">{groupLabel[group.key]}</div>
      {#each group.items as m (m.file)}
        <div class="mod" class:off={!m.enabled}>
          <span>
            <span class="mod-name">{m.name}</span>
            <div class="mod-file">{m.file}{m.enabled ? "" : ".disabled"}</div>
          </span>
          <span class="pill {m.kind}">{t(`mods.pill.${m.kind}`)}</span>
          <span
            class="tgl"
            role="switch"
            tabindex="0"
            aria-checked={m.enabled}
            aria-label={t("mods.toggleAria", { name: m.name })}
            onclick={() => toggle(m)}
            onkeydown={(e) => {
              if (e.key === " " || e.key === "Enter") {
                e.preventDefault();
                toggle(m);
              }
            }}
          ></span>
        </div>
      {/each}
    {/if}
  {/each}
{/if}

<style>
  .mods-head { display: flex; align-items: center; gap: 10px; margin-bottom: 12px; }
  .cnt { font-size: 0.8rem; color: var(--tx-dim); }
  .reset-btn { margin-left: auto; font-size: 0.78rem; font-weight: 700; color: var(--th-tx);
    background: var(--th-soft); border: 1px solid transparent; border-radius: 8px; padding: 7px 12px; }
  .reset-btn + .reset-btn { margin-left: 0; }
  .reset-btn:hover { filter: brightness(1.08); }
  .mod-group { font-size: 0.7rem; letter-spacing: 0.1em; font-weight: 700;
    color: var(--tx-faint); margin: 16px 2px 7px; }
  .mod { display: flex; align-items: center; gap: 12px;
    background: var(--surf); border: 1px solid var(--surf-line);
    border-radius: 9px; padding: 10px 14px; margin-bottom: 6px; }
  .mod.off { opacity: 0.55; }
  .mod-name { font-size: 0.86rem; font-weight: 600; }
  .mod-file { font-size: 0.7rem; color: var(--tx-faint); margin-top: 1px; }
  .pill { font-size: 0.62rem; font-weight: 700; padding: 2px 7px; border-radius: 99px; flex: none; }
  .pill.req { background: var(--surf); border: 1px solid var(--surf-line); color: var(--tx-dim); }
  .pill.opt { background: var(--th-soft); color: var(--th-tx); }
  .pill.user { background: rgba(120, 160, 255, 0.16); color: #7d9ce8; }
</style>
