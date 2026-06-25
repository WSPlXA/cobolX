use crate::cobol::lexer::{clean_name, logical_lines, tokenize};
use crate::cobol::model::{CallKind, ParsedCall, ParsedCopy, ParsedFile, ParsedProgram, Token};
use std::path::Path;

pub(crate) fn parse_source_file(path: &Path) -> std::io::Result<ParsedFile> {
    let content = std::fs::read_to_string(path)?;
    let mut programs = Vec::new();
    let mut copies = Vec::new();
    let mut calls = Vec::new();
    let mut current_program = None::<String>;

    for line in logical_lines(&content) {
        let tokens = tokenize(&line.text);
        if tokens.is_empty() {
            continue;
        }

        for idx in 0..tokens.len() {
            match tokens[idx].text.as_str() {
                "PROGRAM-ID" => {
                    if let Some(name) = tokens.get(idx + 1).map(|t| clean_name(&t.text)) {
                        if !name.is_empty() {
                            current_program = Some(name.clone());
                            programs.push(ParsedProgram {
                                name,
                                start_offset: line.start_offset + tokens[idx].start,
                                byte_len: line.byte_len,
                            });
                        }
                    }
                }
                "COPY" => {
                    if let Some(name) = tokens.get(idx + 1).map(|t| clean_name(&t.text)) {
                        if !name.is_empty() {
                            let replacing_text = tokens
                                .iter()
                                .find(|t| t.text == "REPLACING")
                                .map(|t| line.text[t.start..].trim().to_string());
                            copies.push(ParsedCopy {
                                name,
                                start_offset: line.start_offset + tokens[idx].start,
                                byte_len: line.byte_len,
                                replacing_text,
                            });
                        }
                    }
                }
                "CALL" => {
                    if let Some(target) = tokens.get(idx + 1) {
                        let name = clean_name(&target.text);
                        if !name.is_empty() {
                            let kind = if target.quoted {
                                CallKind::Static
                            } else {
                                CallKind::Dynamic
                            };
                            calls.push(ParsedCall {
                                caller_name: current_program.clone(),
                                target: name,
                                kind,
                                start_offset: line.start_offset + tokens[idx].start,
                                byte_len: line.byte_len,
                                using_count: count_using_args(&tokens[idx + 2..]),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    Ok(ParsedFile {
        path: path.to_path_buf(),
        programs,
        copies,
        calls,
    })
}

fn count_using_args(tokens: &[Token]) -> usize {
    let Some(using_idx) = tokens.iter().position(|t| t.text == "USING") else {
        return 0;
    };

    tokens[using_idx + 1..]
        .iter()
        .take_while(|t| !matches!(t.text.as_str(), "END-CALL" | "RETURNING" | "GIVING"))
        .filter(|t| !matches!(t.text.as_str(), "BY" | "REFERENCE" | "CONTENT" | "VALUE"))
        .count()
}
