// 백엔드 어댑터 — Tauri 커맨드(src-tauri/src/commands.rs)와 1:1.
// Tauri 밖(브라우저 dev)에서는 목 데이터로 폴백한다.
// 아래 목 데이터의 서버 이름/설명은 UI 문자열이 아니라 콘텐츠(데이터)이므로 i18n 대상이 아니다.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { BrowseHit, InstanceView } from "./types";

export const hasTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export interface ProgressPayload {
  id: string;
  stage: "sync" | "loader" | "download" | "java" | "launch";
  item: string;
  done: number;
}

export interface GameExitedPayload {
  id: string;
  code: number | null;
  /** 5분 내 3회 비정상 종료 (§8.12) — true면 E-GM-02 진단 안내 */
  crashLoop: boolean;
}

const MOCK_INSTANCES: InstanceView[] = [
  {
    id: "forest",
    name: "초록 마을 서바이벌",
    initial: "숲",
    manual: false,
    desc: "친목 위주 야생 서버 · 시즌 4 진행 중. 마을 단위 건축과 경제 시스템이 있어요.",
    loaderLabel: "Fabric 1.20.4",
    ver: "v12",
    clog: "Sodium 0.5.8 업데이트, 마을 상점 플러그인 연동 리소스팩 갱신.",
    last: "어제 21:32",
    sync: "동기화 완료 · 오늘 09:12",
    disk: "1.4 GB",
    color: "#4a9b57",
    state: "ok",
    playSize: null,
    domain: "play.forest.kr",
    manifestSource: "https://play.forest.kr/manifest.json",
    order: 0,
    mods: [
      {
        key: "required",
        items: [
          { name: "Sodium", file: "sodium-0.5.8.jar", kind: "req", enabled: true },
          { name: "Fabric API", file: "fabric-api-0.96.11.jar", kind: "req", enabled: true },
          { name: "마을 경제 연동", file: "village-economy-2.1.jar", kind: "req", enabled: true },
        ],
      },
      {
        key: "optional",
        items: [
          { name: "Xaero 미니맵", file: "xaeros-minimap-24.1.jar", kind: "opt", enabled: true },
          { name: "쉐이더 (Iris)", file: "iris-1.6.17.jar", kind: "opt", enabled: false },
        ],
      },
      {
        key: "user",
        items: [{ name: "내가 넣은 인벤토리 정리", file: "inventory-profiles.jar", kind: "user", enabled: true }],
      },
    ],
  },
  {
    id: "tech",
    name: "테크노팩토리 — Create 산업서버",
    initial: "공",
    manual: false,
    desc: "Create 중심 공업 서버. 기차망과 공장 자동화가 메인 콘텐츠입니다.",
    loaderLabel: "NeoForge 1.20.1",
    ver: "v13 대기",
    clog: "Create 대규모 업데이트 + 월드맵 모드 추가 (1.2 GB).",
    last: "3일 전",
    sync: "업데이트 대기 중",
    disk: "3.8 GB",
    color: "#d9932f",
    state: "update",
    playSize: "1.2 GB",
    domain: "play.tech.kr",
    manifestSource: "https://play.tech.kr/manifest.json",
    order: 1,
    mods: [
      {
        key: "required",
        items: [
          { name: "Create", file: "create-0.5.1f.jar", kind: "req", enabled: true },
          { name: "Create: Steam 'n' Rails", file: "snr-1.6.4.jar", kind: "req", enabled: true },
        ],
      },
      { key: "optional", items: [{ name: "JEI", file: "jei-15.3.jar", kind: "opt", enabled: true }] },
      { key: "user", items: [] },
    ],
  },
  {
    id: "lan",
    name: "친구들이랑 야생 (직접 구성)",
    initial: "야",
    manual: true,
    desc: "매니페스트 없이 직접 구성한 인스턴스. 모드는 내가 직접 관리합니다.",
    loaderLabel: "Fabric 1.21",
    ver: "수동",
    clog: "수동 인스턴스에는 매니페스트 동기화가 없습니다. 모든 파일이 사용자 소유입니다.",
    last: "2주 전",
    sync: "수동 인스턴스 · 동기화 없음",
    disk: "0.9 GB",
    color: "#4d94c9",
    state: "ok",
    playSize: null,
    domain: null,
    manifestSource: null,
    order: 2,
    mods: [
      {
        key: "user",
        items: [
          { name: "WorldEdit", file: "worldedit-7.3.jar", kind: "user", enabled: true },
          { name: "Litematica", file: "litematica-0.17.jar", kind: "user", enabled: true },
        ],
      },
    ],
  },
];

const MOCK_BROWSE: Record<string, BrowseHit[]> = {
  modrinth: [
    { name: "Sodium", sub: "렌더링 최적화 · 4.2M", initial: "S", color: "#3a9e6b", file: "sodium-0.5.8.jar", dep: null, optOut: false },
    { name: "Lithium", sub: "틱 최적화 · 2.8M", initial: "L", color: "#5aa8d6", file: "lithium-0.12.1.jar", dep: null, optOut: false },
    { name: "Iris Shaders", sub: "쉐이더 로더 · 3.1M", initial: "I", color: "#7b68c9", file: "iris-1.6.17.jar", dep: "Sodium", optOut: false },
    { name: "ModMenu", sub: "모드 설정 메뉴 · 3.9M", initial: "M", color: "#c98a3d", file: "modmenu-9.0.jar", dep: null, optOut: false },
  ],
  curseforge: [
    { name: "JEI (Just Enough Items)", sub: "아이템/조합법 열람 · 9.7M", initial: "J", color: "#c9564a", file: "jei-15.3.jar", dep: null, optOut: false },
    { name: "JourneyMap", sub: "실시간 지도 · 8.1M", initial: "J", color: "#4a8fc9", file: "journeymap-5.9.jar", dep: null, optOut: false },
    { name: "AppleSkin", sub: "허기/포만도 표시 · 6.4M", initial: "A", color: "#6aa84f", file: "appleskin-2.5.jar", dep: null, optOut: false },
    { name: "어떤 배포제한 모드", sub: "제작자가 API 배포를 제한함", initial: "?", color: "#8a8f9c", file: null, dep: null, optOut: true },
  ],
};

export async function fetchInstances(): Promise<InstanceView[]> {
  if (hasTauri) return await invoke<InstanceView[]>("list_instances");
  return structuredClone(MOCK_INSTANCES);
}

export async function createManualInstance(
  name: string,
  mcVersion: string,
  loaderKind: string,
  color: string,
): Promise<InstanceView | null> {
  if (!hasTauri) return null;
  return await invoke<InstanceView>("create_manual_instance", { name, mcVersion, loaderKind, color });
}

export async function createFromManifest(url: string): Promise<InstanceView | null> {
  if (!hasTauri) return null;
  return await invoke<InstanceView>("create_instance_from_manifest", { url });
}

export interface ManifestPreview {
  name: string;
  desc: string | null;
  domain: string | null;
  modCount: number;
  optionalCount: number;
  totalBytes: number;
  loaderLabel: string;
  mcVersion: string;
}

export async function previewManifest(url: string): Promise<ManifestPreview | null> {
  if (!hasTauri) return null;
  return await invoke<ManifestPreview>("preview_manifest", { url });
}

export async function toggleModBackend(id: string, path: string, enabled: boolean): Promise<void> {
  if (hasTauri) await invoke("toggle_mod", { id, path, enabled });
}

export async function resetBackend(id: string): Promise<void> {
  if (hasTauri) await invoke("reset_instance", { id });
}

export async function playBackend(id: string): Promise<void> {
  if (hasTauri) await invoke("play", { id });
}

export function onProgress(cb: (p: ProgressPayload) => void): void {
  if (hasTauri) void listen<ProgressPayload>("progress", (e) => cb(e.payload));
}

export function onGameExited(cb: (p: GameExitedPayload) => void): void {
  if (hasTauri) void listen<GameExitedPayload>("game-exited", (e) => cb(e.payload));
}

/** 딥링크 설치 (§8.7) — 백엔드에서 스킴/https 검증을 통과한 매니페스트 URL만 수신 */
export function onDeepLinkAdd(cb: (manifestUrl: string) => void): void {
  if (hasTauri) void listen<string>("deeplink-add", (e) => cb(e.payload));
}

export async function searchMods(source: "modrinth" | "curseforge", query: string): Promise<BrowseHit[]> {
  const q = query.trim().toLowerCase();
  return MOCK_BROWSE[source].filter((m) => !q || m.name.toLowerCase().includes(q));
}

export const MC_VERSIONS = ["1.21.1", "1.21", "1.20.6", "1.20.4", "1.20.1", "1.19.2", "1.18.2", "1.16.5"];
export const LOADERS = ["Vanilla", "Fabric", "Quilt", "Forge", "NeoForge"];
export const SWATCH_COLORS = ["#4a9b57", "#d9932f", "#8b6fd8", "#4d94c9", "#c95b7d"];
export const ACCOUNT_NAME = "Steve_KR"; // TODO(M3): MockAuthProvider → 실계정
