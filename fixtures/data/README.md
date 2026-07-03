# 픽스처 데이터

- `manifest.json` — PRD 7.3 스키마의 최소 유효 예시. 빈 파일들의 sha256은 빈 입력의 SHA-256.
- 시나리오 추가 시 `manifest-v2.json` 처럼 버전별 파일을 늘려 diff 테스트에 사용.
- 주의: 픽스처는 http지만 프로덕션 코드는 https만 허용 (PRD 11장). 테스트 빌드에서만 http 예외를 둘 것.
