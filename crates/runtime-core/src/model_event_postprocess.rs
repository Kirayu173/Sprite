use regex::Regex;
use runtime_protocol::memory_citation::MemoryCitation;
use runtime_protocol::memory_citation::MemoryCitationEntry;
use std::collections::HashSet;
use std::sync::OnceLock;

pub trait ModelEventPostProcessor: Send + Sync {
    fn strip_hidden_markup(&self, text: &str, plan_mode: bool) -> String;
    fn parse_memory_citation(&self, citations: Vec<String>) -> Option<MemoryCitation>;
    fn extract_artifact_paths(&self, text: &str) -> Vec<String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultModelEventPostProcessor;

impl ModelEventPostProcessor for DefaultModelEventPostProcessor {
    fn strip_hidden_markup(&self, text: &str, plan_mode: bool) -> String {
        let stripped = strip_known_blocks(text, &["citation_entries", "rollout_ids", "thread_ids"]);
        if plan_mode {
            strip_known_blocks(&stripped, &["proposed_plan"])
        } else {
            stripped
        }
    }

    fn parse_memory_citation(&self, citations: Vec<String>) -> Option<MemoryCitation> {
        parse_memory_citation(citations)
    }

    fn extract_artifact_paths(&self, text: &str) -> Vec<String> {
        artifact_path_regex()
            .captures_iter(text)
            .filter_map(|captures| captures.get(0).map(|match_| match_.as_str().to_string()))
            .collect()
    }
}

fn strip_known_blocks(text: &str, tags: &[&str]) -> String {
    tags.iter()
        .fold(text.to_string(), |current, tag| strip_tag(&current, tag))
}

fn strip_tag(text: &str, tag: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut remaining = text;
    let mut stripped = String::new();

    while let Some(start) = remaining.find(&open) {
        stripped.push_str(&remaining[..start]);
        let after_open = &remaining[start + open.len()..];
        if let Some(end) = after_open.find(&close) {
            remaining = &after_open[end + close.len()..];
        } else {
            remaining = after_open;
            break;
        }
    }

    stripped.push_str(remaining);
    stripped
}

fn parse_memory_citation(citations: Vec<String>) -> Option<MemoryCitation> {
    let mut entries = Vec::new();
    let mut rollout_ids = Vec::new();
    let mut seen_rollout_ids = HashSet::new();

    for citation in citations {
        if let Some(entries_block) =
            extract_block(&citation, "<citation_entries>", "</citation_entries>")
        {
            entries.extend(
                entries_block
                    .lines()
                    .filter_map(parse_memory_citation_entry),
            );
        }

        if let Some(ids_block) = extract_ids_block(&citation) {
            for id in ids_block
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
            {
                if seen_rollout_ids.insert(id.to_string()) {
                    rollout_ids.push(id.to_string());
                }
            }
        }
    }

    if entries.is_empty() && rollout_ids.is_empty() {
        None
    } else {
        Some(MemoryCitation {
            entries,
            rollout_ids,
        })
    }
}

fn parse_memory_citation_entry(line: &str) -> Option<MemoryCitationEntry> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let (location, note) = line.rsplit_once("|note=[")?;
    let note = note.strip_suffix(']')?.trim().to_string();
    let (path, line_range) = location.rsplit_once(':')?;
    let (line_start, line_end) = line_range.split_once('-')?;

    Some(MemoryCitationEntry {
        path: path.trim().to_string(),
        line_start: line_start.trim().parse().ok()?,
        line_end: line_end.trim().parse().ok()?,
        note,
    })
}

fn extract_block<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let (_, rest) = text.split_once(open)?;
    let (body, _) = rest.split_once(close)?;
    Some(body)
}

fn extract_ids_block(text: &str) -> Option<&str> {
    extract_block(text, "<rollout_ids>", "</rollout_ids>")
        .or_else(|| extract_block(text, "<thread_ids>", "</thread_ids>"))
}

fn artifact_path_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"generated_images[/\\][^ \n\r\t"']+\.png"#)
            .expect("artifact path regex should compile")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_hidden_markup_removes_citation_blocks() {
        let processor = DefaultModelEventPostProcessor;
        let text = "hello<citation_entries>\npath:1-2|note=[x]\n</citation_entries>world";

        let stripped = processor.strip_hidden_markup(text, false);

        assert_eq!(stripped, "helloworld");
    }

    #[test]
    fn parse_memory_citation_extracts_entries_and_rollout_ids() {
        let processor = DefaultModelEventPostProcessor;
        let citation = vec![String::from(
            "<citation_entries>\nfoo.rs:10-12|note=[detail]\n</citation_entries><rollout_ids>\nrollout-1\n</rollout_ids>",
        )];

        let parsed = processor.parse_memory_citation(citation).expect("citation");

        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].path, "foo.rs");
        assert_eq!(parsed.rollout_ids, vec!["rollout-1".to_string()]);
    }

    #[test]
    fn extract_artifact_paths_finds_generated_images() {
        let processor = DefaultModelEventPostProcessor;
        let paths = processor.extract_artifact_paths(
            "saved to generated_images/session_1/call_2.png and generated_images\\s\\b.png",
        );

        assert_eq!(
            paths,
            vec![
                "generated_images/session_1/call_2.png".to_string(),
                "generated_images\\s\\b.png".to_string(),
            ]
        );
    }
}
