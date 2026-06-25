use crate::cobol::lexer::{clean_name, logical_lines, tokenize};
use crate::cobol::model::{
    CallKind, CodeBlockKind, ExternalOpKind, ParsedCall, ParsedCodeBlock, ParsedCopy,
    ParsedExternalOp, ParsedFile, ParsedIdentifier, ParsedLiteral, ParsedProgram, Token,
};
use std::path::Path;

pub(crate) fn parse_source_file(path: &Path) -> std::io::Result<ParsedFile> {
    let content = std::fs::read_to_string(path)?;
    let mut programs = Vec::new();
    let mut copies = Vec::new();
    let mut calls = Vec::new();
    let mut code_blocks = Vec::new();
    let mut external_ops = Vec::new();
    let mut identifiers = Vec::new();
    let mut literals = Vec::new();
    let mut current_program = None::<String>;
    let mut in_procedure_division = false;
    let mut current_section = None::<String>;
    let mut current_block_idx = None::<usize>;

    for line in logical_lines(&content) {
        let tokens = tokenize(&line.text);
        if tokens.is_empty() {
            continue;
        }

        if has_two_tokens(&tokens, "PROCEDURE", "DIVISION") {
            in_procedure_division = true;
            current_section = None;
            current_block_idx = None;
            continue;
        }

        if in_procedure_division {
            if let Some(section_name) = parse_section_name(&tokens) {
                code_blocks.push(ParsedCodeBlock {
                    caller_name: current_program.clone(),
                    name: section_name.clone(),
                    kind: CodeBlockKind::Section,
                    parent_section: None,
                    start_offset: line.start_offset + tokens[0].start,
                    byte_len: line.byte_len,
                    statement_count: 0,
                });
                current_section = Some(section_name);
                current_block_idx = Some(code_blocks.len() - 1);
                continue;
            }

            if let Some(paragraph_name) = parse_paragraph_name(&line.text, &tokens) {
                identifiers.push(ParsedIdentifier {
                    caller_name: current_program.clone(),
                    kind: "paragraph_name".to_string(),
                    value: paragraph_name.clone(),
                    start_offset: line.start_offset + tokens[0].start,
                });
                code_blocks.push(ParsedCodeBlock {
                    caller_name: current_program.clone(),
                    name: paragraph_name,
                    kind: CodeBlockKind::Paragraph,
                    parent_section: current_section.clone(),
                    start_offset: line.start_offset + tokens[0].start,
                    byte_len: line.byte_len,
                    statement_count: 0,
                });
                current_block_idx = Some(code_blocks.len() - 1);
                continue;
            }

            if let Some(block_idx) = current_block_idx {
                code_blocks[block_idx].statement_count += 1;
            }
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
                                caller_name: current_program.clone(),
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
                                target: name.clone(),
                                kind,
                                start_offset: line.start_offset + tokens[idx].start,
                                byte_len: line.byte_len,
                                using_count: count_using_args(&tokens[idx + 2..]),
                            });
                            external_ops.push(ParsedExternalOp {
                                caller_name: current_program.clone(),
                                kind: if target.quoted {
                                    ExternalOpKind::CallLiteral
                                } else {
                                    ExternalOpKind::CallIdentifier
                                },
                                verb: "CALL".to_string(),
                                target: Some(name.clone()),
                                start_offset: line.start_offset + tokens[idx].start,
                                byte_len: line.byte_len,
                            });
                            if target.quoted {
                                literals.push(ParsedLiteral {
                                    caller_name: current_program.clone(),
                                    kind: "call_target".to_string(),
                                    value: name,
                                    start_offset: line.start_offset + target.start,
                                });
                            } else {
                                identifiers.push(ParsedIdentifier {
                                    caller_name: current_program.clone(),
                                    kind: "call_target_identifier".to_string(),
                                    value: name,
                                    start_offset: line.start_offset + target.start,
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if !in_procedure_division {
            continue;
        }

        if let Some((op, mut ids, mut lits)) =
            parse_exec_sql(&tokens, line.start_offset, line.byte_len, current_program.clone())
        {
            external_ops.push(op);
            identifiers.append(&mut ids);
            literals.append(&mut lits);
            continue;
        }

        if let Some((op, mut lits)) =
            parse_exec_cics(&tokens, line.start_offset, line.byte_len, current_program.clone())
        {
            external_ops.push(op);
            literals.append(&mut lits);
            continue;
        }

        if let Some((verb, target)) = parse_file_io(&tokens) {
            external_ops.push(ParsedExternalOp {
                caller_name: current_program.clone(),
                kind: ExternalOpKind::FileIo,
                verb: verb.to_string(),
                target: Some(target.clone()),
                start_offset: line.start_offset + tokens[0].start,
                byte_len: line.byte_len,
            });
            identifiers.push(ParsedIdentifier {
                caller_name: current_program.clone(),
                kind: "file_name".to_string(),
                value: target,
                start_offset: line.start_offset + tokens[1].start,
            });
        }
    }

    Ok(ParsedFile {
        path: path.to_path_buf(),
        programs,
        copies,
        calls,
        code_blocks,
        external_ops,
        identifiers,
        literals,
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

fn has_two_tokens(tokens: &[Token], first: &str, second: &str) -> bool {
    tokens.len() >= 2 && tokens[0].text == first && tokens[1].text == second
}

fn parse_section_name(tokens: &[Token]) -> Option<String> {
    (tokens.len() >= 2 && tokens[1].text == "SECTION").then(|| clean_name(&tokens[0].text))
}

fn parse_paragraph_name(line: &str, tokens: &[Token]) -> Option<String> {
    if tokens.len() != 1 || !line.trim_end().ends_with('.') {
        return None;
    }

    let name = clean_name(&tokens[0].text);
    (!name.is_empty() && !is_reserved_label(&name)).then_some(name)
}

fn is_reserved_label(name: &str) -> bool {
    matches!(
        name,
        "ACCEPT"
            | "ADD"
            | "CALL"
            | "CANCEL"
            | "CLOSE"
            | "COMPUTE"
            | "CONTINUE"
            | "DELETE"
            | "DISPLAY"
            | "DIVIDE"
            | "ELSE"
            | "END-CALL"
            | "END-EVALUATE"
            | "END-IF"
            | "END-PERFORM"
            | "ENTRY"
            | "EVALUATE"
            | "EXEC"
            | "EXIT"
            | "GOBACK"
            | "GO"
            | "IF"
            | "INITIALIZE"
            | "INSPECT"
            | "MERGE"
            | "MOVE"
            | "MULTIPLY"
            | "OPEN"
            | "PERFORM"
            | "PROCEDURE"
            | "READ"
            | "RELEASE"
            | "RETURN"
            | "REWRITE"
            | "SEARCH"
            | "SORT"
            | "START"
            | "STOP"
            | "STRING"
            | "SUBTRACT"
            | "UNSTRING"
            | "USE"
            | "WHEN"
            | "WRITE"
    )
}

fn parse_exec_sql(
    tokens: &[Token],
    line_start_offset: usize,
    byte_len: usize,
    caller_name: Option<String>,
) -> Option<(ParsedExternalOp, Vec<ParsedIdentifier>, Vec<ParsedLiteral>)> {
    if !has_two_tokens(tokens, "EXEC", "SQL") {
        return None;
    }

    let verb = tokens[2..]
        .iter()
        .find(|t| matches!(t.text.as_str(), "SELECT" | "INSERT" | "UPDATE" | "DELETE" | "MERGE"))
        .map(|t| t.text.clone())
        .unwrap_or_else(|| "SQL".to_string());
    let target = extract_sql_target(tokens, &verb);
    let mut identifiers = Vec::new();
    if let Some(ref table) = target {
        identifiers.push(ParsedIdentifier {
            caller_name: caller_name.clone(),
            kind: "sql_table".to_string(),
            value: table.clone(),
            start_offset: line_start_offset,
        });
    }
    let literals = tokens
        .iter()
        .filter(|t| t.quoted)
        .map(|t| ParsedLiteral {
            caller_name: caller_name.clone(),
            kind: "string_literal".to_string(),
            value: clean_name(&t.text),
            start_offset: line_start_offset + t.start,
        })
        .collect::<Vec<_>>();

    Some((
        ParsedExternalOp {
            caller_name,
            kind: ExternalOpKind::ExecSql,
            verb,
            target,
            start_offset: line_start_offset,
            byte_len,
        },
        identifiers,
        literals,
    ))
}

fn extract_sql_target(tokens: &[Token], verb: &str) -> Option<String> {
    match verb {
        "SELECT" => token_after_keyword(tokens, "FROM"),
        "INSERT" => token_after_keyword(tokens, "INTO"),
        "UPDATE" => token_after_keyword(tokens, "UPDATE"),
        "DELETE" => token_after_keyword(tokens, "FROM"),
        "MERGE" => token_after_keyword(tokens, "INTO")
            .or_else(|| token_after_keyword(tokens, "USING")),
        _ => None,
    }
}

fn parse_exec_cics(
    tokens: &[Token],
    line_start_offset: usize,
    byte_len: usize,
    caller_name: Option<String>,
) -> Option<(ParsedExternalOp, Vec<ParsedLiteral>)> {
    if !has_two_tokens(tokens, "EXEC", "CICS") {
        return None;
    }

    let verb = tokens
        .get(2)
        .map(|t| clean_name(&t.text))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "CICS".to_string());
    let target_idx = tokens.iter().position(|t| t.text == "PROGRAM");
    let target = target_idx
        .and_then(|idx| tokens.get(idx + 1))
        .map(|t| clean_name(&t.text))
        .filter(|s| !s.is_empty());
    let literals = target_idx
        .and_then(|idx| tokens.get(idx + 1))
        .filter(|t| t.quoted)
        .map(|t| ParsedLiteral {
            caller_name: caller_name.clone(),
            kind: "exec_cics_program".to_string(),
            value: clean_name(&t.text),
            start_offset: line_start_offset + t.start,
        })
        .into_iter()
        .collect::<Vec<_>>();

    Some((
        ParsedExternalOp {
            caller_name,
            kind: ExternalOpKind::ExecCics,
            verb,
            target,
            start_offset: line_start_offset,
            byte_len,
        },
        literals,
    ))
}

fn parse_file_io(tokens: &[Token]) -> Option<(String, String)> {
    match tokens.first()?.text.as_str() {
        "OPEN" => {
            let mode_idx = tokens.iter().position(|t| {
                matches!(t.text.as_str(), "INPUT" | "OUTPUT" | "I-O" | "EXTEND")
            })?;
            let target = tokens.get(mode_idx + 1).map(|t| clean_name(&t.text))?;
            (!target.is_empty()).then_some(("OPEN".to_string(), target))
        }
        "READ" | "WRITE" | "REWRITE" | "DELETE" | "START" | "CLOSE" | "RETURN" | "RELEASE" => {
            let target = tokens.get(1).map(|t| clean_name(&t.text))?;
            (!target.is_empty()).then_some((tokens[0].text.clone(), target))
        }
        _ => None,
    }
}

fn token_after_keyword(tokens: &[Token], keyword: &str) -> Option<String> {
    let idx = tokens.iter().position(|t| t.text == keyword)?;
    tokens
        .get(idx + 1)
        .map(|t| clean_name(&t.text))
        .filter(|s| !s.is_empty())
}
