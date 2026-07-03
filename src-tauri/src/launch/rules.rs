//! version JSON `rules` 배열 평가 — TD-01 §2.
//!
//! 의미론: rules가 비어 있으면 허용. 있으면 기본 거부에서 시작해 순서대로 평가,
//! **마지막으로 매치된 rule의 action**이 최종 결정.
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Allow,
    Disallow,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub action: RuleAction,
    #[serde(default)]
    pub os: Option<OsRule>,
    #[serde(default)]
    pub features: Option<BTreeMap<String, bool>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct OsRule {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
    /// OS 버전 정규식. 지원 하한(Win10+/mac12+)상 항상 충족으로 간주 (TD-01 §2).
    #[serde(default)]
    pub version: Option<String>,
}

/// 평가 문맥. os_name: "windows" | "osx", arch: "x86_64" | "arm64".
/// features: 런처가 명시적으로 켠 feature 키 집합 — 미지의 feature는 false (TD-01 §2).
#[derive(Debug, Clone)]
pub struct RuleContext {
    pub os_name: String,
    pub arch: String,
    pub features: BTreeSet<String>,
}

impl RuleContext {
    pub fn new(os_name: &str, arch: &str) -> Self {
        RuleContext {
            os_name: os_name.into(),
            arch: arch.into(),
            features: BTreeSet::new(),
        }
    }
}

pub fn evaluate(rules: &[Rule], ctx: &RuleContext) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if rule_matches(rule, ctx) {
            allowed = rule.action == RuleAction::Allow;
        }
    }
    allowed
}

fn rule_matches(rule: &Rule, ctx: &RuleContext) -> bool {
    if let Some(os) = &rule.os {
        if let Some(name) = &os.name {
            if name != &ctx.os_name {
                return false;
            }
        }
        if let Some(arch) = &os.arch {
            if arch != &ctx.arch {
                return false;
            }
        }
        // os.version 정규식은 지원 하한상 항상 충족으로 간주 (TD-01 §2)
    }
    if let Some(features) = &rule.features {
        for (key, required) in features {
            if ctx.features.contains(key) != *required {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win() -> RuleContext {
        RuleContext::new("windows", "x86_64")
    }
    fn mac_arm() -> RuleContext {
        RuleContext::new("osx", "arm64")
    }
    fn mac_x64() -> RuleContext {
        RuleContext::new("osx", "x86_64")
    }

    fn rules(json: &str) -> Vec<Rule> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn empty_rules_allow() {
        assert!(evaluate(&[], &win()));
    }

    #[test]
    fn allow_os_only_matches_that_os() {
        let r = rules(r#"[{"action":"allow","os":{"name":"osx"}}]"#);
        assert!(evaluate(&r, &mac_arm()));
        assert!(evaluate(&r, &mac_x64()));
        assert!(!evaluate(&r, &win()), "no matching rule -> default disallow");
    }

    #[test]
    fn allow_all_then_disallow_osx_excludes_mac() {
        let r = rules(
            r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]"#,
        );
        assert!(evaluate(&r, &win()));
        assert!(!evaluate(&r, &mac_arm()), "last matching rule wins");
    }

    #[test]
    fn arch_rule_matches_exact_arch_only() {
        // TD-01 §2 매트릭스: x86 rule은 win-x64에 비매치, arm64 rule은 mac-aarch64에만 매치
        let x86 = rules(r#"[{"action":"allow","os":{"arch":"x86"}}]"#);
        assert!(!evaluate(&x86, &win()));
        let arm = rules(r#"[{"action":"allow","os":{"name":"osx","arch":"arm64"}}]"#);
        assert!(evaluate(&arm, &mac_arm()));
        assert!(!evaluate(&arm, &mac_x64()));
    }

    #[test]
    fn os_version_regex_is_treated_as_satisfied() {
        let r = rules(r#"[{"action":"allow","os":{"name":"windows","version":"^10\\."}}]"#);
        assert!(evaluate(&r, &win()));
    }

    #[test]
    fn feature_rules_require_explicit_feature() {
        let r = rules(r#"[{"action":"allow","features":{"has_quick_plays_support":true}}]"#);
        assert!(!evaluate(&r, &win()), "unknown feature is false");
        let mut ctx = win();
        ctx.features.insert("has_quick_plays_support".into());
        assert!(evaluate(&r, &ctx));
        // false 요구 조건: feature가 꺼져 있어야 매치
        let neg = rules(r#"[{"action":"allow","features":{"is_demo_user":false}}]"#);
        assert!(evaluate(&neg, &win()));
    }
}
