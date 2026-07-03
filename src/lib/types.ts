// 프론트 뷰 모델 — 백엔드 instance.json/lockfile(PRD 7장)을 UI 관점으로 요약한 형태.
// Tauri 커맨드 결선 시 src-tauri 쪽 직렬화 타입과 1:1로 맞춘다.

export type InstanceState = "ok" | "update" | "dirty" | "offline";
export type ModKind = "req" | "opt" | "user";
export type GroupKey = "required" | "optional" | "user";

export interface ModItem {
  name: string;
  file: string;
  kind: ModKind;
  enabled: boolean;
}

export interface ModGroup {
  key: GroupKey;
  items: ModItem[];
}

export interface InstanceView {
  id: string;
  name: string;
  initial: string;
  manual: boolean;
  desc: string;
  loaderLabel: string;
  ver: string;
  clog: string;
  last: string;
  sync: string;
  disk: string;
  /** 서버 테마 대표색 (PRD 8.5) */
  color: string;
  state: InstanceState;
  playSize: string | null;
  domain: string | null;
  /** 최근 플레이 정렬 키 */
  order: number;
  mods: ModGroup[];
}

export interface BrowseHit {
  name: string;
  sub: string;
  initial: string;
  color: string;
  file: string | null;
  /** 자동 설치될 의존성 이름 (§8.17) */
  dep: string | null;
  /** CF 배포 opt-out — 런처 내 다운로드 불가 (E-MB-01) */
  optOut: boolean;
}
