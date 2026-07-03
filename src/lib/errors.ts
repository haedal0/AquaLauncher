// §9 에러 처리 — 백엔드 AppError(src-tauri/src/error.rs)의 {code, detail} 페이로드를
// 정규화하고, 코드별 사용자 문구(i18n 키)와 복구 액션 가능 여부를 정의한다.
export interface AppErrorPayload {
  code: string;
  detail?: unknown;
}

/** 에러 다이얼로그 상태 — retry는 호출측이 원 작업 재실행 클로저를 넘긴다. */
export interface UiError extends AppErrorPayload {
  retry?: () => void;
}

const KNOWN_CODES = new Set([
  "E-MF-01", "E-MF-02", "E-MF-03", "E-MF-04", "E-MF-05",
  "E-AU-01", "E-AU-02", "E-AU-03", "E-AU-04", "E-AU-05", "E-AU-06",
  "E-JV-01", "E-LD-01", "E-DL-01", "E-MB-01",
  "E-GM-01", "E-GM-02", "E-IN-01",
]);

/** §9에서 재시도 버튼을 제공하는 코드. E-MF-02/03/05 등은 재시도가 무의미하다. */
const RETRYABLE = new Set([
  "E-MF-01", "E-MF-04", "E-AU-01", "E-JV-01", "E-LD-01", "E-DL-01", "E-IN-01",
]);

/** invoke 거부 페이로드를 AppError 형태로 정규화 — 알 수 없는 형태는 E-IN-01. */
export function normalizeError(e: unknown): AppErrorPayload {
  if (e && typeof e === "object" && "code" in e && typeof (e as { code: unknown }).code === "string") {
    return e as AppErrorPayload;
  }
  return { code: "E-IN-01", detail: e instanceof Error ? e.message : String(e) };
}

export function errorMessageKey(code: string): string {
  return KNOWN_CODES.has(code) ? `err.${code}` : "err.E-IN-01";
}

export function isRetryable(code: string): boolean {
  return RETRYABLE.has(code) || !KNOWN_CODES.has(code);
}

/** detail을 "상세 오류 복사"용 텍스트로 — E-MF-04는 실패 파일 목록(부분 재시도 안내). */
export function detailText(p: AppErrorPayload): string {
  const d = p.detail;
  if (d == null) return "";
  if (typeof d === "string") return d;
  if (typeof d === "object" && "files" in d && Array.isArray((d as { files: unknown }).files)) {
    return (d as { files: string[] }).files.join("\n");
  }
  return JSON.stringify(d);
}
