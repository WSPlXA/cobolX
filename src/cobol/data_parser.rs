use crate::cobol::copybook::resolve_copybook;
use crate::cobol::lexer::{clean_name, logical_lines, tokenize};
use crate::cobol::model::{LogicalLine, ParsedDataItem, Token};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct DataParseState {
    in_data_division: bool,
    section: Option<String>,
    parent_stack: Vec<(u16, String)>,
    items: Vec<ParsedDataItem>,
}

pub(crate) fn collect_data_items(
    root: &Path,
    path: &Path,
    copybook_index: &HashMap<String, Vec<PathBuf>>,
    depth: usize,
) -> std::io::Result<Vec<ParsedDataItem>> {
    let mut state = DataParseState::default();
    parse_data_file(root, path, copybook_index, depth, false, &mut state)?;
    Ok(state.items)
}

fn parse_data_file(
    root: &Path,
    path: &Path,
    copybook_index: &HashMap<String, Vec<PathBuf>>,
    depth: usize,
    is_copybook: bool,
    state: &mut DataParseState,
) -> std::io::Result<()> {
    if depth > 16 {
        return Ok(());
    }

    let content = std::fs::read_to_string(path)?;
    let mut copybook_local_state;
    let state = if is_copybook && !state.in_data_division {
        copybook_local_state = DataParseState {
            in_data_division: true,
            section: None,
            parent_stack: Vec::new(),
            items: Vec::new(),
        };
        &mut copybook_local_state
    } else {
        state
    };

    for line in logical_lines(&content) {
        let tokens = tokenize(&line.text);
        if tokens.is_empty() {
            continue;
        }

        if has_two_tokens(&tokens, "DATA", "DIVISION") {
            state.in_data_division = true;
            state.parent_stack.clear();
            continue;
        }
        if has_two_tokens(&tokens, "PROCEDURE", "DIVISION") {
            if !is_copybook {
                break;
            }
            continue;
        }
        if !state.in_data_division && !is_copybook {
            continue;
        }
        if is_section_line(&tokens) {
            state.section = Some(tokens[0].text.clone());
            state.parent_stack.clear();
            continue;
        }
        if tokens[0].text == "COPY" {
            if let Some(copy_name) = tokens.get(1).map(|t| clean_name(&t.text)) {
                if let Some(copy_path) = resolve_copybook(root, path, &copy_name, copybook_index) {
                    parse_data_file(root, &copy_path, copybook_index, depth + 1, true, state)?;
                }
            }
            continue;
        }

        if let Some(item) = parse_data_item_line(path, &line, &tokens, state) {
            state.items.push(item);
        }
    }

    Ok(())
}

fn parse_data_item_line(
    path: &Path,
    line: &LogicalLine,
    tokens: &[Token],
    state: &mut DataParseState,
) -> Option<ParsedDataItem> {
    let level = tokens.first()?.text.parse::<u16>().ok()?;
    if !is_data_level(level) {
        return None;
    }
    let name = tokens.get(1).map(|t| clean_name(&t.text))?;
    if name.is_empty() {
        return None;
    }

    while state
        .parent_stack
        .last()
        .is_some_and(|(parent_level, _)| *parent_level >= level)
    {
        state.parent_stack.pop();
    }
    let parent_name = if matches!(level, 1 | 66 | 77) {
        None
    } else {
        state.parent_stack.last().map(|(_, name)| name.clone())
    };

    let pic = extract_clause_text(
        &line.text,
        tokens,
        &["PIC", "PICTURE"],
        &[
            "USAGE",
            "OCCURS",
            "REDEFINES",
            "VALUE",
            "VALUES",
            "SIGN",
            "SYNC",
            "SYNCHRONIZED",
            "JUST",
            "JUSTIFIED",
            "DISPLAY",
            "BINARY",
            "COMP",
            "COMP-1",
            "COMP-2",
            "COMP-3",
            "COMP-4",
            "COMP-5",
            "COMPUTATIONAL",
            "COMPUTATIONAL-1",
            "COMPUTATIONAL-2",
            "COMPUTATIONAL-3",
            "COMPUTATIONAL-4",
            "COMPUTATIONAL-5",
            "PACKED-DECIMAL",
            "INDEX",
            "POINTER",
            "NATIONAL",
        ],
    );
    let usage_clause = extract_usage_clause(&line.text, tokens);
    let occurs = extract_occurs(tokens);
    let redefines = extract_next_name(tokens, "REDEFINES");

    if !matches!(level, 66 | 88) {
        state.parent_stack.push((level, name.clone()));
    }

    Some(ParsedDataItem {
        source_path: path.to_path_buf(),
        name,
        level,
        parent_name,
        pic,
        usage_clause,
        occurs,
        redefines,
        section: state.section.clone(),
        byte_offset: None,
        byte_size: None,
        storage_kind: None,
        layout_status: None,
        start_offset: line.start_offset + tokens[0].start,
        byte_len: line.byte_len,
    })
}

fn is_data_level(level: u16) -> bool {
    (1..=49).contains(&level) || matches!(level, 66 | 77 | 88)
}

fn has_two_tokens(tokens: &[Token], first: &str, second: &str) -> bool {
    tokens.len() >= 2 && tokens[0].text == first && tokens[1].text == second
}

fn is_section_line(tokens: &[Token]) -> bool {
    tokens.len() >= 2
        && tokens[1].text == "SECTION"
        && matches!(
            tokens[0].text.as_str(),
            "FILE" | "WORKING-STORAGE" | "LOCAL-STORAGE" | "LINKAGE"
        )
}

fn extract_clause_text(
    line: &str,
    tokens: &[Token],
    names: &[&str],
    stop_keywords: &[&str],
) -> Option<String> {
    let idx = tokens
        .iter()
        .position(|t| names.iter().any(|name| t.text == *name))?;
    let mut start_idx = idx + 1;
    if tokens.get(start_idx).is_some_and(|t| t.text == "IS") {
        start_idx += 1;
    }
    let start = tokens.get(start_idx)?.start;
    let end = tokens[start_idx..]
        .iter()
        .find(|t| stop_keywords.iter().any(|keyword| t.text == *keyword))
        .map(|t| t.start)
        .unwrap_or_else(|| line.len());
    Some(
        line[start..end]
            .trim()
            .trim_end_matches('.')
            .trim()
            .to_ascii_uppercase(),
    )
    .filter(|s| !s.is_empty())
}

fn extract_usage_clause(line: &str, tokens: &[Token]) -> Option<String> {
    let explicit = extract_clause_text(
        line,
        tokens,
        &["USAGE"],
        &[
            "OCCURS",
            "REDEFINES",
            "VALUE",
            "VALUES",
            "SIGN",
            "SYNC",
            "SYNCHRONIZED",
            "JUST",
            "JUSTIFIED",
        ],
    );
    if explicit.is_some() {
        return explicit;
    }

    tokens
        .iter()
        .find(|t| {
            matches!(
                t.text.as_str(),
                "DISPLAY"
                    | "BINARY"
                    | "COMP"
                    | "COMP-1"
                    | "COMP-2"
                    | "COMP-3"
                    | "COMP-4"
                    | "COMP-5"
                    | "COMPUTATIONAL"
                    | "COMPUTATIONAL-1"
                    | "COMPUTATIONAL-2"
                    | "COMPUTATIONAL-3"
                    | "COMPUTATIONAL-4"
                    | "COMPUTATIONAL-5"
                    | "PACKED-DECIMAL"
                    | "INDEX"
                    | "POINTER"
                    | "NATIONAL"
            )
        })
        .map(|t| t.text.clone())
}

fn extract_occurs(tokens: &[Token]) -> Option<i64> {
    let idx = tokens.iter().position(|t| t.text == "OCCURS")?;
    tokens
        .iter()
        .skip(idx + 1)
        .find_map(|t| t.text.parse::<i64>().ok())
}

fn extract_next_name(tokens: &[Token], keyword: &str) -> Option<String> {
    let idx = tokens.iter().position(|t| t.text == keyword)?;
    tokens.get(idx + 1).map(|t| clean_name(&t.text))
}
