//! 인자 치환 + 접속 인자 버전 분기 — TD-01 §6, §7.
//!
//! 마스킹: 비밀 값이 치환된 인자의 인덱스를 함께 반환한다. 로그 출력은
//! 반드시 `SubstitutedArgs::display_masked()`를 거친다 (PRD §8.15-5).
use std::collections::BTreeMap;

/// 치환 문맥. secrets에 등록된 키의 값은 마스킹 대상.
#[derive(Debug, Default)]
pub struct SubstContext {
    values: BTreeMap<String, String>,
    secret_keys: Vec<String>,
}

impl SubstContext {
    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        self.values.insert(key.into(), value.into());
    }
    pub fn set_secret(&mut self, key: &str, value: impl Into<String>) {
        self.values.insert(key.into(), value.into());
        self.secret_keys.push(key.into());
    }
}

#[derive(Debug)]
pub struct SubstitutedArgs {
    pub args: Vec<String>,
    /// 비밀 값이 포함된 args 인덱스 — 로그 출력 시 치환
    pub masked_indices: Vec<usize>,
}

impl SubstitutedArgs {
    /// 로그/디버그용: 마스킹 대상 인자를 "***"로 대체한 사본
    pub fn display_masked(&self) -> Vec<String> {
        let mut out = self.args.clone();
        for &i in &self.masked_indices {
            if let Some(a) = out.get_mut(i) {
                *a = "***".into();
            }
        }
        out
    }
}

/// `${key}` 플레이스홀더 치환. 미지의 키는 빈 문자열 + 경고 로그 (TD-01 §6).
pub fn substitute(args: &[String], ctx: &SubstContext) -> SubstitutedArgs {
    let mut out = Vec::with_capacity(args.len());
    let mut masked_indices = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        let mut result = String::with_capacity(arg.len());
        let mut rest = arg.as_str();
        let mut has_secret = false;
        while let Some(start) = rest.find("${") {
            result.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            match after.find('}') {
                Some(end) => {
                    let key = &after[..end];
                    match ctx.values.get(key) {
                        Some(value) => {
                            result.push_str(value);
                            if ctx.secret_keys.iter().any(|k| k == key) {
                                has_secret = true;
                            }
                        }
                        None => {
                            tracing::warn!(placeholder = key, "unknown placeholder, substituting empty");
                        }
                    }
                    rest = &after[end + 1..];
                }
                None => {
                    // 닫히지 않은 `${` — 원문 유지
                    result.push_str(&rest[start..]);
                    rest = "";
                }
            }
        }
        result.push_str(rest);
        if has_secret {
            masked_indices.push(i);
        }
        out.push(result);
    }
    SubstitutedArgs { args: out, masked_indices }
}

/// 릴리스 버전 "1.X[.Y]" 파싱. 스냅샷 등은 None (TD-01 §7).
fn parse_release(mc_version: &str) -> Option<(u32, u32)> {
    let mut it = mc_version.split('.');
    let major: u32 = it.next()?.parse().ok()?;
    let minor: u32 = it.next()?.parse().ok()?;
    if let Some(patch) = it.next() {
        let _: u32 = patch.parse().ok()?;
    }
    Some((major, minor))
}

/// 서버 자동 접속 인자 — PRD §8.6 버전 분기 필수.
/// 1.20+: --quickPlayMultiplayer "host:port" / 1.13~1.19: --server --port.
/// 버전 파싱 불가(스냅샷 등): 빈 벡터 + 경고 (TD-01 §7).
pub fn join_args(mc_version: &str, host: &str, port: u16) -> Vec<String> {
    match parse_release(mc_version) {
        Some((major, minor)) if (major, minor) >= (1, 20) => vec![
            "--quickPlayMultiplayer".into(),
            format!("{host}:{port}"),
        ],
        Some(_) => vec![
            "--server".into(),
            host.into(),
            "--port".into(),
            port.to_string(),
        ],
        None => {
            tracing::warn!(mc_version, "cannot parse MC version, skipping join args");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> SubstContext {
        let mut c = SubstContext::default();
        c.set("auth_player_name", "Steve_KR");
        c.set("game_directory", "/inst/abc");
        c.set_secret("auth_access_token", "SECRET-TOKEN-123");
        c
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn substitutes_known_placeholders() {
        let out = substitute(
            &s(&["--username", "${auth_player_name}", "--gameDir", "${game_directory}"]),
            &ctx(),
        );
        assert_eq!(out.args, s(&["--username", "Steve_KR", "--gameDir", "/inst/abc"]));
        assert!(out.masked_indices.is_empty());
    }

    #[test]
    fn multiple_placeholders_in_one_arg() {
        let out = substitute(&s(&["-Dpath=${game_directory}/mods,${auth_player_name}"]), &ctx());
        assert_eq!(out.args, s(&["-Dpath=/inst/abc/mods,Steve_KR"]));
    }

    #[test]
    fn unknown_placeholder_becomes_empty() {
        let out = substitute(&s(&["--clientId", "${clientid}"]), &ctx());
        assert_eq!(out.args, s(&["--clientId", ""]));
    }

    #[test]
    fn secret_substitution_is_tracked_and_masked() {
        let out = substitute(&s(&["--accessToken", "${auth_access_token}"]), &ctx());
        assert_eq!(out.args[1], "SECRET-TOKEN-123");
        assert_eq!(out.masked_indices, [1]);
        let shown = out.display_masked().join(" ");
        assert!(
            !shown.contains("SECRET-TOKEN-123"),
            "token must never appear in log output"
        );
    }

    #[test]
    fn quickplay_for_1_20_and_later() {
        assert_eq!(
            join_args("1.20", "play.example.com", 25565),
            s(&["--quickPlayMultiplayer", "play.example.com:25565"])
        );
        assert_eq!(
            join_args("1.20.4", "play.example.com", 25565),
            s(&["--quickPlayMultiplayer", "play.example.com:25565"])
        );
        assert_eq!(
            join_args("1.21.1", "h", 1),
            s(&["--quickPlayMultiplayer", "h:1"])
        );
    }

    #[test]
    fn legacy_server_args_for_1_13_to_1_19() {
        assert_eq!(
            join_args("1.19.4", "play.example.com", 25565),
            s(&["--server", "play.example.com", "--port", "25565"])
        );
        assert_eq!(
            join_args("1.13", "h", 25565),
            s(&["--server", "h", "--port", "25565"])
        );
    }

    #[test]
    fn unparseable_version_yields_no_join_args() {
        assert!(join_args("23w13a_or_b", "h", 25565).is_empty());
    }
}
