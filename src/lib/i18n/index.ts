// i18n — PRD 8.13. 프론트 문자열 하드코딩 금지 규칙의 진입점.
// 1차 언어: 한국어, 영어. `{name}` 형태의 단순 보간 지원.
import ko from "./ko.json";
import en from "./en.json";

type Dict = Record<string, string>;
const dicts: Record<string, Dict> = { ko, en };
let locale: string = "ko";

export function setLocale(l: string) {
  if (dicts[l]) locale = l;
}

export function t(key: string, params?: Record<string, string | number>): string {
  let s = dicts[locale][key] ?? dicts["en"][key] ?? key;
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      s = s.replaceAll(`{${k}}`, String(v));
    }
  }
  return s;
}
