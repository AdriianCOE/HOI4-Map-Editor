//! Read-only railway and supply-node map inputs.
//!
//! The HOI4-compatible paths are fixed map files rather than `default.map`
//! entries. These compact models intentionally own no edit or save state so
//! later reference discovery can consume the parsed province references.

use crate::app::state::TextSpan;

use super::{ProjectSources, ResolvedSource, SourceLookup};

pub const RAILWAYS_PATH: &str = "map/railways.txt";
pub const SUPPLY_NODES_PATH: &str = "map/supply_nodes.txt";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Railway {
    pub source: ResolvedSource,
    pub line_number: usize,
    pub level: u32,
    pub declared_province_count: u32,
    pub provinces: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplyNode {
    pub source: ResolvedSource,
    pub line_number: usize,
    pub level: u32,
    pub province_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogisticsInput {
    Railway,
    SupplyNode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogisticsLoadIssue {
    pub input: LogisticsInput,
    pub source: Option<ResolvedSource>,
    pub line_number: Option<usize>,
    pub span: Option<TextSpan>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RailwayLoadResult {
    pub source: Option<ResolvedSource>,
    pub railways: Vec<Railway>,
    pub issues: Vec<LogisticsLoadIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SupplyNodeLoadResult {
    pub source: Option<ResolvedSource>,
    pub supply_nodes: Vec<SupplyNode>,
    pub issues: Vec<LogisticsLoadIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LogisticsLoadResult {
    pub railways: RailwayLoadResult,
    pub supply_nodes: SupplyNodeLoadResult,
}

pub fn load_logistics(sources: &ProjectSources) -> LogisticsLoadResult {
    LogisticsLoadResult {
        railways: load_railways(sources),
        supply_nodes: load_supply_nodes(sources),
    }
}

fn load_railways(sources: &ProjectSources) -> RailwayLoadResult {
    let mut result = RailwayLoadResult::default();
    let Some((source, text)) = read_input(sources, LogisticsInput::Railway, &mut result.issues)
    else {
        return result;
    };
    result.source = Some(source.clone());
    let mut offset = 0;
    for (line_index, raw_line) in text.split_inclusive('\n').enumerate() {
        let line = raw_line.trim_end_matches(['\r', '\n']);
        let line_number = line_index + 1;
        let Some((content, content_start)) = content(line) else {
            offset += raw_line.len();
            continue;
        };
        let values = match parse_values(content) {
            Ok(values) if values.len() >= 3 => values,
            Ok(_) => {
                result.issues.push(issue(LogisticsInput::Railway, &source, line_number, offset + content_start, content.len(), "railway entry requires level, declared province count, and at least one province"));
                offset += raw_line.len();
                continue;
            }
            Err(error) => {
                result.issues.push(issue(
                    LogisticsInput::Railway,
                    &source,
                    line_number,
                    offset + content_start,
                    content.len(),
                    &format!("invalid railway entry: {error}"),
                ));
                offset += raw_line.len();
                continue;
            }
        };
        result.railways.push(Railway {
            source: source.clone(),
            line_number,
            level: values[0],
            declared_province_count: values[1],
            provinces: values[2..].to_vec(),
        });
        offset += raw_line.len();
    }
    result
}

fn load_supply_nodes(sources: &ProjectSources) -> SupplyNodeLoadResult {
    let mut result = SupplyNodeLoadResult::default();
    let Some((source, text)) = read_input(sources, LogisticsInput::SupplyNode, &mut result.issues)
    else {
        return result;
    };
    result.source = Some(source.clone());
    let mut offset = 0;
    for (line_index, raw_line) in text.split_inclusive('\n').enumerate() {
        let line = raw_line.trim_end_matches(['\r', '\n']);
        let line_number = line_index + 1;
        let Some((content, content_start)) = content(line) else {
            offset += raw_line.len();
            continue;
        };
        let values = match parse_values(content) {
            Ok(values) if values.len() >= 2 => values,
            Ok(_) => {
                result.issues.push(issue(
                    LogisticsInput::SupplyNode,
                    &source,
                    line_number,
                    offset + content_start,
                    content.len(),
                    "supply node entry requires level and province",
                ));
                offset += raw_line.len();
                continue;
            }
            Err(error) => {
                result.issues.push(issue(
                    LogisticsInput::SupplyNode,
                    &source,
                    line_number,
                    offset + content_start,
                    content.len(),
                    &format!("invalid supply node entry: {error}"),
                ));
                offset += raw_line.len();
                continue;
            }
        };
        result.supply_nodes.push(SupplyNode {
            source: source.clone(),
            line_number,
            level: values[0],
            province_id: values[1],
        });
        offset += raw_line.len();
    }
    result
}

fn read_input(
    sources: &ProjectSources,
    input: LogisticsInput,
    issues: &mut Vec<LogisticsLoadIssue>,
) -> Option<(ResolvedSource, String)> {
    let path = match input {
        LogisticsInput::Railway => RAILWAYS_PATH,
        LogisticsInput::SupplyNode => SUPPLY_NODES_PATH,
    };
    let source = match sources.resolve(path) {
        Ok(SourceLookup::Found(source)) => source,
        Ok(SourceLookup::NotFound | SourceLookup::BlockedByReplacePath { .. }) => return None,
        Err(error) => {
            issues.push(LogisticsLoadIssue {
                input,
                source: None,
                line_number: None,
                span: None,
                message: format!("cannot resolve {path}: {error}"),
            });
            return None;
        }
    };
    let bytes = match sources.read_resolved(&source) {
        Ok(bytes) => bytes,
        Err(error) => {
            issues.push(LogisticsLoadIssue {
                input,
                source: Some(source),
                line_number: None,
                span: None,
                message: format!("cannot read {path}: {error}"),
            });
            return None;
        }
    };
    match String::from_utf8(bytes) {
        Ok(text) => Some((source, text)),
        Err(error) => {
            issues.push(LogisticsLoadIssue {
                input,
                source: Some(source),
                line_number: None,
                span: None,
                message: format!("{path} is not UTF-8: {error}"),
            });
            None
        }
    }
}

fn content(line: &str) -> Option<(&str, usize)> {
    let uncommented = line.split_once('#').map_or(line, |(content, _)| content);
    let content = uncommented.trim();
    (!content.is_empty()).then_some((content, line.find(content).unwrap_or_default()))
}

fn parse_values(content: &str) -> Result<Vec<u32>, String> {
    content
        .split_whitespace()
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|_| format!("'{value}' is not an unsigned integer"))
        })
        .collect()
}

fn issue(
    input: LogisticsInput,
    source: &ResolvedSource,
    line_number: usize,
    start: usize,
    len: usize,
    message: &str,
) -> LogisticsLoadIssue {
    LogisticsLoadIssue {
        input,
        source: Some(source.clone()),
        line_number: Some(line_number),
        span: Some(TextSpan::new(start, len)),
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_whitespace_and_sparse_ids_are_parsed_without_dense_assumptions() {
        assert_eq!(
            content("  1  4  1 7 42 500 # trunk"),
            Some(("1  4  1 7 42 500", 2))
        );
        assert_eq!(
            parse_values("1 4 1 7 42 500").unwrap(),
            vec![1, 4, 1, 7, 42, 500]
        );
        assert!(parse_values("1 -1 7").is_err());
    }
}
