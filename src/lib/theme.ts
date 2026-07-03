// 서버 테마 파생 — PRD 8.5 / 8.14.
// 대표색 하나에서 라이트/다크 모드별 배경 혼합값을 파생한다 (mockup.html applyThemeVars와 동일).
export function applyThemeVars(color: string, light: boolean) {
  const r = document.documentElement.style;
  r.setProperty("--th", color);
  if (light) {
    r.setProperty("--th-soft", `color-mix(in srgb, ${color} 16%, transparent)`);
    r.setProperty("--th-tx", `color-mix(in srgb, ${color} 82%, black)`);
    r.setProperty("--th-bg1", `color-mix(in srgb, ${color} 5%, #f2f3f6)`);
    r.setProperty("--th-bg2", `color-mix(in srgb, ${color} 15%, #e9ebf0)`);
  } else {
    r.setProperty("--th-soft", `color-mix(in srgb, ${color} 18%, transparent)`);
    r.setProperty("--th-tx", `color-mix(in srgb, ${color} 45%, white)`);
    r.setProperty("--th-bg1", `color-mix(in srgb, ${color} 12%, #0b0d10)`);
    r.setProperty("--th-bg2", `color-mix(in srgb, ${color} 24%, #13161b)`);
  }
}

export function softOf(color: string): string {
  return `color-mix(in srgb, ${color} 18%, transparent)`;
}
