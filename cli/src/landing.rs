//! 딥링크 랜딩 페이지 생성 — PRD 8.7 / 10.1.
//!
//! 운영자는 `aqualauncher://` 원시 링크 대신 이 https 랜딩 페이지를 배포한다.
//! 페이지는 런처 실행(딥링크)을 시도하고, 일정 시간 내 포커스 전환이 없으면
//! 런처 다운로드 안내를 표시한다 (미설치 사용자 대응).
use aqua_manifest::urlpolicy::validate_distribution_url;

/// 딥링크 쿼리 파라미터용 percent-encoding — RFC 3986 unreserved 외 전부 인코딩.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// HTML 텍스트/속성 이스케이프 — 매니페스트의 이름/설명은 외부 입력이다 (§11).
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// 랜딩 페이지 HTML 렌더. manifest_url은 https만 허용 (§8.7).
pub fn render(
    server_name: &str,
    description: Option<&str>,
    manifest_url: &str,
    download_url: Option<&str>,
) -> Result<String, String> {
    validate_distribution_url(manifest_url).map_err(|e| e.to_string())?;
    if let Some(d) = download_url {
        validate_distribution_url(d).map_err(|e| e.to_string())?;
    }
    let deep_link = format!("aqualauncher://add?manifest={}", urlencode(manifest_url));
    let name = escape_html(server_name);
    let desc = escape_html(description.unwrap_or(""));
    let dl_section = match download_url {
        Some(d) => format!(
            r#"<a class="btn dl" href="{}">AquaLauncher 다운로드</a>"#,
            escape_html(d)
        ),
        None => r#"<span class="dl-hint">서버 운영자에게 런처 다운로드 안내를 요청하세요.</span>"#
            .to_string(),
    };
    Ok(format!(
        r#"<!doctype html>
<html lang="ko">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{name} — AquaLauncher로 참여</title>
<style>
  :root {{ color-scheme: dark; }}
  body {{ margin: 0; min-height: 100vh; display: grid; place-items: center;
    font-family: system-ui, "Apple SD Gothic Neo", "Malgun Gothic", sans-serif;
    background: #0d0f14; color: #e8eaf0; }}
  .card {{ max-width: 420px; width: 92%; background: #161922; border: 1px solid #262b38;
    border-radius: 16px; padding: 30px 28px; text-align: center; }}
  h1 {{ font-size: 1.3rem; margin: 0 0 8px; }}
  p.desc {{ color: #9aa1b2; font-size: 0.9rem; line-height: 1.6; margin: 0 0 20px; }}
  .btn {{ display: inline-block; background: #4a9b57; color: #fff; font-weight: 700;
    padding: 12px 28px; border-radius: 10px; text-decoration: none; }}
  .btn.dl {{ background: #2f6fb2; margin-top: 10px; }}
  #fallback {{ display: none; margin-top: 22px; padding-top: 18px; border-top: 1px solid #262b38;
    font-size: 0.85rem; color: #9aa1b2; }}
  #fallback.show {{ display: block; }}
  .dl-hint {{ display: inline-block; margin-top: 6px; }}
  code {{ background: #0d0f14; border-radius: 6px; padding: 2px 6px; font-size: 0.78rem;
    word-break: break-all; }}
</style>
</head>
<body>
<main class="card">
  <h1>{name}</h1>
  <p class="desc">{desc}</p>
  <a class="btn" id="open" href="{deep_link}">AquaLauncher로 서버 추가</a>
  <div id="fallback">
    <p>런처가 열리지 않았나요? AquaLauncher가 설치되어 있지 않을 수 있습니다.</p>
    {dl_section}
    <p>설치 후 이 페이지의 버튼을 다시 누르거나, 런처의 "인스턴스 추가"에 아래 주소를 붙여넣으세요.</p>
    <code>{manifest_url_escaped}</code>
  </div>
</main>
<script>
  // 딥링크 실행 시도 후 일정 시간 내 포커스 전환(런처로 이동)이 없으면 다운로드 안내 (PRD 8.7)
  var attempted = false;
  function attempt() {{
    attempted = true;
    var left = false;
    var onHide = function () {{ left = true; }};
    document.addEventListener("visibilitychange", onHide, {{ once: true }});
    window.addEventListener("blur", onHide, {{ once: true }});
    setTimeout(function () {{
      if (!left) document.getElementById("fallback").classList.add("show");
    }}, 1800);
  }}
  document.getElementById("open").addEventListener("click", attempt);
</script>
</body>
</html>
"#,
        manifest_url_escaped = escape_html(manifest_url),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_urlencoded_deep_link() {
        let html =
            render("초록 마을", None, "https://play.forest.kr/manifest.json", None).unwrap();
        assert!(html
            .contains("aqualauncher://add?manifest=https%3A%2F%2Fplay.forest.kr%2Fmanifest.json"));
    }

    #[test]
    fn rejects_non_https_manifest_url() {
        assert!(render("s", None, "http://evil.example/m.json", None).is_err());
        assert!(render("s", None, "https://ok.example/m.json", Some("http://evil.example/dl"))
            .is_err());
    }

    /// §11: 매니페스트의 이름/설명은 외부 입력 — 스크립트 주입 차단.
    #[test]
    fn escapes_html_in_name_and_description() {
        let html = render(
            "<script>alert(1)</script>",
            Some("a & b \"c\""),
            "https://play.example.com/manifest.json",
            None,
        )
        .unwrap();
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(html.contains("a &amp; b &quot;c&quot;"));
    }

    #[test]
    fn download_button_only_when_url_given() {
        let with = render("s", None, "https://e.com/m.json", Some("https://e.com/dl")).unwrap();
        assert!(with.contains("AquaLauncher 다운로드"));
        let without = render("s", None, "https://e.com/m.json", None).unwrap();
        assert!(without.contains("다운로드 안내를 요청"));
    }
}
