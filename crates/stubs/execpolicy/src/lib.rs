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

    pub fn check<F>(&self, tokens: &[String], _fallback: &F) -> Evaluation
    where
        F: Fn(&[String]) -> Evaluation,
    {
        let Some(program) = tokens.first() else {
            return Evaluation {
                decision: Decision::Allow,
                matched_rules: Vec::new(),
            };
        };
        let Some(rules) = self.rules.get_vec(program) else {
            return Evaluation {
                decision: Decision::Allow,
                matched_rules: Vec::new(),
            };
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
        Evaluation {
            decision: Decision::Allow,
            matched_rules: Vec::new(),
        }
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
