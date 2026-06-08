use multimap::MultiMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Prompt,
    Forbidden,
}

pub mod rule {
    use super::Decision;
    use std::sync::Arc;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum PatternToken {
        Single(String),
        Alts(Vec<String>),
    }

    impl PatternToken {
        pub fn alternatives(&self) -> Vec<String> {
            match self {
                Self::Single(value) => vec![value.clone()],
                Self::Alts(values) => values.clone(),
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PrefixPattern {
        pub first: Arc<str>,
        pub rest: Arc<[PatternToken]>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PrefixRule {
        pub pattern: PrefixPattern,
        pub decision: Decision,
        pub justification: Option<String>,
    }
}

pub type RuleRef = Arc<rule::PrefixRule>;

#[derive(Debug, Clone, Default)]
pub struct Policy {
    rules: MultiMap<String, RuleRef>,
}

impl Policy {
    pub fn new(rules: MultiMap<String, RuleRef>) -> Self {
        Self { rules }
    }

    pub fn rules(&self) -> &MultiMap<String, RuleRef> {
        &self.rules
    }

    pub fn check<F>(&self, tokens: &[String], fallback: &F) -> Evaluation
    where
        F: Fn(&[String]) -> Evaluation,
    {
        let Some(program) = tokens.first() else {
            return fallback(tokens);
        };
        let Some(rules) = self.rules.get_vec(program) else {
            return fallback(tokens);
        };
        for rule in rules {
            let rest_matches = rule
                .pattern
                .rest
                .iter()
                .zip(tokens.iter().skip(1))
                .all(|(pattern, token)| pattern.alternatives().iter().any(|alt| alt == token));
            if rest_matches && rule.pattern.rest.len() <= tokens.len().saturating_sub(1) {
                let mut matched_prefix = Vec::with_capacity(rule.pattern.rest.len() + 1);
                matched_prefix.push(rule.pattern.first.to_string());
                matched_prefix.extend(tokens.iter().skip(1).take(rule.pattern.rest.len()).cloned());
                return Evaluation {
                    decision: rule.decision,
                    matched_rules: vec![RuleMatch::PrefixRuleMatch {
                        matched_prefix,
                        decision: rule.decision,
                        resolved_program: None,
                        justification: rule.justification.clone(),
                    }],
                };
            }
        }
        fallback(tokens)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluation {
    pub decision: Decision,
    pub matched_rules: Vec<RuleMatch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleMatch {
    PrefixRuleMatch {
        matched_prefix: Vec<String>,
        decision: Decision,
        resolved_program: Option<String>,
        justification: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::PatternToken;
    use crate::rule::PrefixPattern;
    use crate::rule::PrefixRule;

    fn fallback(_: &[String]) -> Evaluation {
        Evaluation {
            decision: Decision::Prompt,
            matched_rules: Vec::new(),
        }
    }

    #[test]
    fn check_uses_matching_prefix_rule() {
        let rule = Arc::new(PrefixRule {
            pattern: PrefixPattern {
                first: Arc::from("cargo"),
                rest: Arc::from([PatternToken::Single("test".to_string())]),
            },
            decision: Decision::Allow,
            justification: Some("tests are allowed".to_string()),
        });
        let mut rules = MultiMap::new();
        rules.insert("cargo".to_string(), rule);
        let policy = Policy::new(rules);

        let evaluation = policy.check(&["cargo".to_string(), "test".to_string()], &fallback);

        assert_eq!(evaluation.decision, Decision::Allow);
        assert_eq!(
            evaluation.matched_rules,
            vec![RuleMatch::PrefixRuleMatch {
                matched_prefix: vec!["cargo".to_string(), "test".to_string()],
                decision: Decision::Allow,
                resolved_program: None,
                justification: Some("tests are allowed".to_string()),
            }]
        );
    }

    #[test]
    fn check_delegates_to_fallback_without_match() {
        let policy = Policy::default();

        let evaluation = policy.check(&["cargo".to_string(), "fmt".to_string()], &fallback);

        assert_eq!(evaluation.decision, Decision::Prompt);
        assert!(evaluation.matched_rules.is_empty());
    }
}
