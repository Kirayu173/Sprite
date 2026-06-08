use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use multimap::MultiMap;
use utils_absolute_path::AbsolutePathBuf;

use crate::decision::Decision;
use crate::error::Error;
use crate::error::ErrorLocation;
use crate::error::Result;
use crate::error::TextPosition;
use crate::error::TextRange;
use crate::executable_name::executable_lookup_key;
use crate::executable_name::executable_path_lookup_key;
use crate::rule::NetworkRule;
use crate::rule::NetworkRuleProtocol;
use crate::rule::PatternToken;
use crate::rule::PrefixPattern;
use crate::rule::PrefixRule;
use crate::rule::RuleRef;
use crate::rule::validate_match_examples;
use crate::rule::validate_not_match_examples;

pub struct PolicyParser {
    builder: PolicyBuilder,
}

impl Default for PolicyParser {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicyParser {
    pub fn new() -> Self {
        Self {
            builder: PolicyBuilder::new(),
        }
    }

    pub fn parse(&mut self, policy_identifier: &str, contents: &str) -> Result<()> {
        for call in parse_calls(policy_identifier, contents)? {
            match call.name.as_str() {
                "prefix_rule" => self.parse_prefix_rule(call)?,
                "network_rule" => self.parse_network_rule(call)?,
                "host_executable" => self.parse_host_executable(call)?,
                other => {
                    return Err(Error::Parse(format!(
                        "unsupported execpolicy builtin `{other}`"
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn build(self) -> crate::policy::Policy {
        self.builder.build()
    }

    fn parse_prefix_rule(&mut self, call: ParsedCall) -> Result<()> {
        let pattern = call
            .args
            .get("pattern")
            .ok_or_else(|| Error::InvalidRule("prefix_rule requires pattern".to_string()))?;
        let pattern_tokens = parse_pattern(pattern)?;

        let decision = call
            .args
            .get("decision")
            .map(|raw| parse_string(raw).and_then(|value| Decision::parse(&value)))
            .transpose()?
            .unwrap_or(Decision::Allow);
        let justification = call
            .args
            .get("justification")
            .map(|raw| parse_string(raw))
            .transpose()?;
        if justification
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(Error::InvalidRule(
                "justification cannot be empty".to_string(),
            ));
        }

        let matches = call
            .args
            .get("match")
            .map(|raw| parse_examples(raw))
            .transpose()?
            .unwrap_or_default();
        let not_matches = call
            .args
            .get("not_match")
            .map(|raw| parse_examples(raw))
            .transpose()?
            .unwrap_or_default();

        let (first_token, remaining_tokens) = pattern_tokens
            .split_first()
            .ok_or_else(|| Error::InvalidPattern("pattern cannot be empty".to_string()))?;
        let rest: Arc<[PatternToken]> = remaining_tokens.to_vec().into();
        let rules: Vec<RuleRef> = first_token
            .alternatives()
            .iter()
            .map(|head| {
                Arc::new(PrefixRule {
                    pattern: PrefixPattern {
                        first: Arc::from(head.as_str()),
                        rest: rest.clone(),
                    },
                    decision,
                    justification: justification.clone(),
                }) as RuleRef
            })
            .collect();

        let policy = crate::policy::Policy::from_parts(
            rules
                .iter()
                .map(|rule| (rule.program().to_string(), rule.clone()))
                .collect(),
            Vec::new(),
            self.builder.host_executables_by_name.clone(),
        );
        let location = Some(call.location);
        validate_not_match_examples(&policy, &rules, &not_matches)
            .map_err(|err| attach_validation_location(err, location.clone()))?;
        validate_match_examples(&policy, &rules, &matches)
            .map_err(|err| attach_validation_location(err, location))?;

        for rule in rules {
            self.builder.add_rule(rule);
        }
        Ok(())
    }

    fn parse_network_rule(&mut self, call: ParsedCall) -> Result<()> {
        let host = parse_required_string(&call, "host")?;
        let protocol = NetworkRuleProtocol::parse(&parse_required_string(&call, "protocol")?)?;
        let decision = parse_network_rule_decision(&parse_required_string(&call, "decision")?)?;
        let justification = call
            .args
            .get("justification")
            .map(|raw| parse_string(raw))
            .transpose()?;
        if justification
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(Error::InvalidRule(
                "justification cannot be empty".to_string(),
            ));
        }
        self.builder.add_network_rule(NetworkRule {
            host: crate::rule::normalize_network_rule_host(&host)?,
            protocol,
            decision,
            justification,
        });
        Ok(())
    }

    fn parse_host_executable(&mut self, call: ParsedCall) -> Result<()> {
        let name = parse_required_string(&call, "name")?;
        validate_host_executable_name(&name)?;
        let paths = call
            .args
            .get("paths")
            .ok_or_else(|| Error::InvalidRule("host_executable requires paths".to_string()))?;
        let paths = parse_string_array(paths)?
            .into_iter()
            .map(|raw| parse_literal_absolute_path(&raw, &name))
            .collect::<Result<Vec<_>>>()?;
        self.builder
            .add_host_executable(executable_lookup_key(&name), paths);
        Ok(())
    }
}

#[derive(Debug)]
struct PolicyBuilder {
    rules_by_program: MultiMap<String, RuleRef>,
    network_rules: Vec<NetworkRule>,
    host_executables_by_name: HashMap<String, Arc<[AbsolutePathBuf]>>,
}

impl PolicyBuilder {
    fn new() -> Self {
        Self {
            rules_by_program: MultiMap::new(),
            network_rules: Vec::new(),
            host_executables_by_name: HashMap::new(),
        }
    }

    fn add_rule(&mut self, rule: RuleRef) {
        self.rules_by_program
            .insert(rule.program().to_string(), rule);
    }

    fn add_network_rule(&mut self, rule: NetworkRule) {
        self.network_rules.push(rule);
    }

    fn add_host_executable(&mut self, name: String, paths: Vec<AbsolutePathBuf>) {
        self.host_executables_by_name.insert(name, paths.into());
    }

    fn build(self) -> crate::policy::Policy {
        crate::policy::Policy::from_parts(
            self.rules_by_program,
            self.network_rules,
            self.host_executables_by_name,
        )
    }
}

#[derive(Debug)]
struct ParsedCall {
    name: String,
    args: HashMap<String, String>,
    location: ErrorLocation,
}

fn parse_calls(policy_identifier: &str, contents: &str) -> Result<Vec<ParsedCall>> {
    let mut calls = Vec::new();
    for (line_index, line) in contents.lines().enumerate() {
        let line_without_comment = strip_comment(line).trim();
        if line_without_comment.is_empty() {
            continue;
        }
        let Some(open) = line_without_comment.find('(') else {
            return Err(Error::Parse(format!(
                "expected builtin call at line {}",
                line_index + 1
            )));
        };
        let Some(close) = line_without_comment.rfind(')') else {
            return Err(Error::Parse(format!(
                "missing closing ')' at line {}",
                line_index + 1
            )));
        };
        if close < open || !line_without_comment[close + 1..].trim().is_empty() {
            return Err(Error::Parse(format!(
                "unexpected trailing tokens at line {}",
                line_index + 1
            )));
        }
        let name = line_without_comment[..open].trim();
        if name.is_empty() {
            return Err(Error::Parse(format!(
                "missing builtin name at line {}",
                line_index + 1
            )));
        }
        let args = parse_call_args(&line_without_comment[open + 1..close])?;
        calls.push(ParsedCall {
            name: name.to_string(),
            args,
            location: ErrorLocation {
                path: policy_identifier.to_string(),
                range: TextRange {
                    start: TextPosition {
                        line: line_index + 1,
                        column: 1,
                    },
                    end: TextPosition {
                        line: line_index + 1,
                        column: line.chars().count().max(1),
                    },
                },
            },
        });
    }
    Ok(calls)
}

fn strip_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == '#' => return &line[..index],
            None => {}
        }
    }
    line
}

fn parse_call_args(raw: &str) -> Result<HashMap<String, String>> {
    let mut args = HashMap::new();
    for part in split_top_level(raw, ',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some(eq) = find_top_level_char(part, '=') else {
            return Err(Error::Parse(format!("expected key=value argument: {part}")));
        };
        let key = part[..eq].trim();
        if key.is_empty() {
            return Err(Error::Parse("argument key cannot be empty".to_string()));
        }
        if args
            .insert(key.to_string(), part[eq + 1..].trim().to_string())
            .is_some()
        {
            return Err(Error::Parse(format!("duplicate argument `{key}`")));
        }
    }
    Ok(args)
}

fn split_top_level(raw: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in raw.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == '[' => depth += 1,
            None if ch == ']' => depth -= 1,
            None if ch == delimiter && depth == 0 => {
                parts.push(&raw[start..index]);
                start = index + ch.len_utf8();
            }
            None => {}
        }
    }
    parts.push(&raw[start..]);
    parts
}

fn find_top_level_char(raw: &str, target: char) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in raw.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == '[' => depth += 1,
            None if ch == ']' => depth -= 1,
            None if ch == target && depth == 0 => return Some(index),
            None => {}
        }
    }
    None
}

fn parse_required_string(call: &ParsedCall, key: &str) -> Result<String> {
    call.args
        .get(key)
        .ok_or_else(|| Error::InvalidRule(format!("{} requires {key}", call.name)))
        .and_then(|raw| parse_string(raw))
}

fn parse_string(raw: &str) -> Result<String> {
    serde_json::from_str::<String>(raw)
        .or_else(|_| parse_single_quoted_string(raw))
        .map_err(|_| Error::Parse(format!("expected string literal: {raw}")))
}

fn parse_single_quoted_string(raw: &str) -> std::result::Result<String, ()> {
    let raw = raw.trim();
    let Some(inner) = raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) else {
        return Err(());
    };
    Ok(inner.replace("\\'", "'").replace("\\\\", "\\"))
}

fn parse_pattern(raw: &str) -> Result<Vec<PatternToken>> {
    let items = parse_array_items(raw)?;
    if items.is_empty() {
        return Err(Error::InvalidPattern("pattern cannot be empty".to_string()));
    }
    items
        .into_iter()
        .map(|item| {
            if item.trim_start().starts_with('[') {
                let alternatives = parse_string_array(&item)?;
                match alternatives.as_slice() {
                    [] => Err(Error::InvalidPattern(
                        "pattern alternatives cannot be empty".to_string(),
                    )),
                    [single] => Ok(PatternToken::Single(single.clone())),
                    _ => Ok(PatternToken::Alts(alternatives)),
                }
            } else {
                Ok(PatternToken::Single(parse_string(&item)?))
            }
        })
        .collect()
}

fn parse_examples(raw: &str) -> Result<Vec<Vec<String>>> {
    parse_array_items(raw)?
        .into_iter()
        .map(|item| {
            if item.trim_start().starts_with('[') {
                let tokens = parse_string_array(&item)?;
                if tokens.is_empty() {
                    Err(Error::InvalidExample(
                        "example cannot be an empty list".to_string(),
                    ))
                } else {
                    Ok(tokens)
                }
            } else {
                parse_string_example(&parse_string(&item)?)
            }
        })
        .collect()
}

fn parse_string_example(raw: &str) -> Result<Vec<String>> {
    let tokens = shlex::split(raw).ok_or_else(|| {
        Error::InvalidExample("example string has invalid shell syntax".to_string())
    })?;

    if tokens.is_empty() {
        Err(Error::InvalidExample(
            "example cannot be an empty string".to_string(),
        ))
    } else {
        Ok(tokens)
    }
}

fn parse_string_array(raw: &str) -> Result<Vec<String>> {
    parse_array_items(raw)?
        .into_iter()
        .map(|item| parse_string(&item))
        .collect()
}

fn parse_array_items(raw: &str) -> Result<Vec<String>> {
    let raw = raw.trim();
    let Some(inner) = raw.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return Err(Error::Parse(format!("expected array literal: {raw}")));
    };
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    Ok(split_top_level(inner, ',')
        .into_iter()
        .map(|part| part.trim().to_string())
        .collect())
}

fn parse_literal_absolute_path(raw: &str, name: &str) -> Result<AbsolutePathBuf> {
    if !Path::new(raw).is_absolute() {
        return Err(Error::InvalidRule(format!(
            "host_executable paths must be absolute (got {raw})"
        )));
    }
    let path = AbsolutePathBuf::try_from(raw.to_string())
        .map_err(|error| Error::InvalidRule(format!("invalid absolute path `{raw}`: {error}")))?;
    let Some(path_name) = executable_path_lookup_key(path.as_path()) else {
        return Err(Error::InvalidRule(format!(
            "host_executable path `{raw}` must have basename `{name}`"
        )));
    };
    if path_name != executable_lookup_key(name) {
        return Err(Error::InvalidRule(format!(
            "host_executable path `{raw}` must have basename `{name}`"
        )));
    }
    Ok(path)
}

fn validate_host_executable_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::InvalidRule(
            "host_executable name cannot be empty".to_string(),
        ));
    }

    let path = Path::new(name);
    if path.components().count() != 1
        || path.file_name().and_then(|value| value.to_str()) != Some(name)
    {
        return Err(Error::InvalidRule(format!(
            "host_executable name must be a bare executable name (got {name})"
        )));
    }

    Ok(())
}

fn parse_network_rule_decision(raw: &str) -> Result<Decision> {
    match raw {
        "deny" => Ok(Decision::Forbidden),
        other => Decision::parse(other),
    }
}

fn attach_validation_location(error: Error, location: Option<ErrorLocation>) -> Error {
    match location {
        Some(location) => error.with_location(location),
        None => error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MatchOptions;

    fn fallback(_: &[String]) -> Decision {
        Decision::Prompt
    }

    #[test]
    fn parses_prefix_rule() {
        let mut parser = PolicyParser::new();
        parser
            .parse(
                "test.rules",
                r#"prefix_rule(pattern=["cargo", ["test", "check"]], decision="allow")"#,
            )
            .unwrap();
        let policy = parser.build();

        assert_eq!(
            policy
                .check(&["cargo".into(), "test".into()], &fallback)
                .decision,
            Decision::Allow
        );
        assert_eq!(
            policy
                .check(&["cargo".into(), "fmt".into()], &fallback)
                .decision,
            Decision::Prompt
        );
    }

    #[test]
    fn parses_network_and_host_executable_rules() {
        let mut parser = PolicyParser::new();
        let path = if cfg!(windows) {
            r#"C:\\Windows\\System32\\curl.exe"#
        } else {
            "/usr/bin/curl"
        };
        parser
            .parse(
                "test.rules",
                &format!(
                    r#"
network_rule(host="API.GitHub.com", protocol="https", decision="deny")
host_executable(name="curl", paths=["{path}"])
prefix_rule(pattern=["curl"], decision="allow")
"#
                ),
            )
            .unwrap();
        let policy = parser.build();

        assert_eq!(policy.network_rules().len(), 1);
        assert_eq!(policy.network_rules()[0].host, "api.github.com");
        assert!(policy.host_executables().contains_key("curl"));
        let evaluation = policy.check_with_options(
            &[path.into()],
            &fallback,
            &MatchOptions {
                resolve_host_executables: true,
            },
        );
        assert_eq!(evaluation.decision, Decision::Allow);
    }
}
