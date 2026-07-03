// 사람이 읽는 용량 표기 — UI 전용 (단위 기호는 i18n 대상 아님).
export function fmtBytes(n: number): string {
  if (n >= 1 << 30) return `${(n / (1 << 30)).toFixed(1)} GB`;
  if (n >= 1 << 20) return `${(n / (1 << 20)).toFixed(1)} MB`;
  return `${n} B`;
}
