use crate::cobol::copybook::{build_copybook_index, resolve_copybook};
use crate::cobol::data_parser::collect_data_items;
use crate::cobol::layout::compute_physical_layout;
use crate::cobol::lexer::clean_name;
use crate::cobol::model::ParsedCodeBlock;
pub use crate::cobol::model::{
    CallKind, CallSummary, CopybookSummary, IndexReport, ProgramSummary,
};
use crate::cobol::scanner::{CobolFileType, scan_sandbox};
use crate::cobol::source_parser::parse_source_file;
use crate::memory::MemoryStore;
use rusqlite::params;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::hash::Hash;
use std::path::Path;
use std::time::UNIX_EPOCH;

type IndexResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub fn index_sandbox(root: &Path, store: &mut MemoryStore) -> IndexResult<IndexReport> {
    let files = scan_sandbox(root)?;
    let source_count = files
        .iter()
        .filter(|f| f.file_type == CobolFileType::Source)
        .count();
    let copybook_count = files.len() - source_count;

    let parsed = files
        .iter()
        .filter(|f| f.file_type == CobolFileType::Source)
        .map(|f| parse_source_file(&f.path))
        .collect::<Result<Vec<_>, _>>()?;

    let tx = store.connection_mut().transaction()?;
    tx.execute_batch(
        r#"
        DELETE FROM external_ops;
        DELETE FROM code_blocks;
        DELETE FROM literals;
        DELETE FROM identifiers;
        DELETE FROM program_features;
        DELETE FROM copybook_features;
        DELETE FROM call_edges;
        DELETE FROM copybook_uses;
        DELETE FROM data_items;
        DELETE FROM programs;
        DELETE FROM files;
        "#,
    )?;

    let mut file_ids = HashMap::with_capacity(files.len());
    for file in &files {
        let rel = relative_path(root, &file.path);
        let kind = match file.file_type {
            CobolFileType::Source => "source",
            CobolFileType::Copybook => "copybook",
        };
        tx.execute(
            "INSERT INTO files(path, kind, size_bytes, mtime_unix, sha256) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                rel,
                kind,
                file.size_bytes as i64,
                mtime_unix(&file.path),
                Option::<Vec<u8>>::None
            ],
        )?;
        file_ids.insert(file.path.clone(), tx.last_insert_rowid());
    }

    let mut program_ids = HashMap::new();
    let mut program_file = HashMap::new();
    for file in &parsed {
        let Some(file_id) = file_ids.get(&file.path).copied() else {
            continue;
        };
        for program in &file.programs {
            tx.execute(
                "INSERT INTO programs(name, file_id, start_offset, byte_len) VALUES (?1, ?2, ?3, ?4)",
                params![
                    program.name,
                    file_id,
                    program.start_offset as i64,
                    program.byte_len as i64
                ],
            )?;
            let id = tx.last_insert_rowid();
            program_ids.insert(program.name.clone(), id);
            program_file.insert(program.name.clone(), file.path.clone());
        }
    }

    let mut report_programs = HashMap::<String, ProgramSummary>::new();
    for (name, path) in &program_file {
        report_programs.insert(
            name.clone(),
            ProgramSummary {
                name: name.clone(),
                path: path.clone(),
                copybooks: Vec::new(),
                calls: Vec::new(),
                data_items: 0,
            },
        );
    }

    let copybook_index = build_copybook_index(&files);
    let mut copybook_uses = 0;
    let mut resolved_copybooks = 0;
    let mut unresolved_copybooks = Vec::new();
    let mut static_calls = 0;
    let mut dynamic_calls = 0;
    let mut data_items = 0usize;

    let mut incoming_calls = HashMap::<i64, usize>::new();
    let mut outgoing_calls = HashMap::<i64, usize>::new();
    let mut static_calls_by_program = HashMap::<i64, usize>::new();
    let mut dynamic_calls_by_program = HashMap::<i64, usize>::new();
    let mut copybook_use_count_by_program = HashMap::<i64, usize>::new();
    let mut distinct_copybooks_by_program = HashMap::<i64, HashSet<i64>>::new();
    let mut referenced_by_files = HashMap::<i64, HashSet<i64>>::new();
    let mut data_item_count_by_program = HashMap::<i64, usize>::new();
    let mut paragraph_count_by_program = HashMap::<i64, usize>::new();
    let mut external_op_count_by_program = HashMap::<i64, usize>::new();
    let mut identifier_count_by_program = HashMap::<i64, usize>::new();
    let mut literal_count_by_program = HashMap::<i64, usize>::new();
    let mut copybook_programs = HashMap::<i64, HashSet<i64>>::new();
    let mut copybook_files = HashMap::<i64, HashSet<i64>>::new();
    let mut copybook_replacing_counts = HashMap::<i64, usize>::new();
    let mut copybook_item_names = HashMap::<i64, Vec<String>>::new();
    let mut identifier_mentions = HashMap::<MentionKey, MentionAggregate>::new();
    let mut literal_mentions = HashMap::<MentionKey, MentionAggregate>::new();

    for file in &parsed {
        let Some(from_file_id) = file_ids.get(&file.path).copied() else {
            continue;
        };
        let default_program = file.programs.first().map(|p| p.name.as_str());

        for copy in &file.copies {
            copybook_uses += 1;
            let resolved = resolve_copybook(root, &file.path, &copy.name, &copybook_index);
            if resolved.is_some() {
                resolved_copybooks += 1;
            } else {
                unresolved_copybooks.push(format!(
                    "{} from {}",
                    copy.name,
                    relative_path(root, &file.path)
                ));
            }

            let resolved_file_id = resolved.as_ref().and_then(|p| file_ids.get(p).copied());
            tx.execute(
                "INSERT INTO copybook_uses(from_file_id, copybook_name, start_offset, byte_len, resolved_file_id, resolve_status, replacing_text) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    from_file_id,
                    copy.name,
                    copy.start_offset as i64,
                    copy.byte_len as i64,
                    resolved_file_id,
                    if resolved_file_id.is_some() { "resolved" } else { "missing" },
                    copy.replacing_text,
                ],
            )?;

            let caller_name = copy.caller_name.as_deref().or(default_program);
            if let Some(caller_name) = caller_name {
                if let Some(summary) = report_programs.get_mut(caller_name) {
                    summary.copybooks.push(CopybookSummary {
                        name: copy.name.clone(),
                        resolved_path: resolved.clone(),
                        has_replacing: copy.replacing_text.is_some(),
                    });
                }
                if let Some(program_id) = program_ids.get(caller_name).copied() {
                    increment_count(&mut copybook_use_count_by_program, program_id);
                    if let Some(copybook_file_id) = resolved_file_id {
                        distinct_copybooks_by_program
                            .entry(program_id)
                            .or_default()
                            .insert(copybook_file_id);
                        copybook_programs
                            .entry(copybook_file_id)
                            .or_default()
                            .insert(program_id);
                        copybook_files
                            .entry(copybook_file_id)
                            .or_default()
                            .insert(from_file_id);
                        if copy.replacing_text.is_some() {
                            increment_count(&mut copybook_replacing_counts, copybook_file_id);
                        }
                    }
                }
            }
        }

        if let Some(program_name) = default_program {
            let Some(program_id) = program_ids.get(program_name).copied() else {
                continue;
            };
            let mut expanded_items = collect_data_items(root, &file.path, &copybook_index, 0)?;
            compute_physical_layout(&mut expanded_items);
            data_items += expanded_items.len();

            for item in expanded_items {
                let source_file_id = file_ids.get(&item.source_path).copied();
                tx.execute(
                    "INSERT INTO data_items(program_id, source_file_id, name, level, parent_name, pic, usage_clause, occurs, redefines, section, byte_offset, byte_size, storage_kind, layout_status, start_offset, byte_len) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                    params![
                        program_id,
                        source_file_id,
                        item.name,
                        item.level as i64,
                        item.parent_name,
                        item.pic,
                        item.usage_clause,
                        item.occurs,
                        item.redefines,
                        item.section,
                        item.byte_offset,
                        item.byte_size,
                        item.storage_kind,
                        item.layout_status,
                        item.start_offset as i64,
                        item.byte_len as i64,
                    ],
                )?;

                increment_count(&mut data_item_count_by_program, program_id);
                increment_count(&mut identifier_count_by_program, program_id);
                if let Some(source_file_id) = source_file_id {
                    aggregate_mention(
                        &mut identifier_mentions,
                        program_id,
                        source_file_id,
                        "data_name",
                        &item.name,
                        item.start_offset,
                    );
                    if source_file_id != from_file_id {
                        copybook_item_names
                            .entry(source_file_id)
                            .or_default()
                            .push(item.name.clone());
                    }
                }
                if let Some(summary) = report_programs.get_mut(program_name) {
                    summary.data_items += 1;
                }
            }
        }

        for code_block in &file.code_blocks {
            let Some(program_id) = resolve_program_id(
                &program_ids,
                code_block.caller_name.as_deref(),
                default_program,
            ) else {
                continue;
            };
            tx.execute(
                "INSERT INTO code_blocks(program_id, source_file_id, name, kind, parent_section, sequence_no, statement_count, start_offset, byte_len) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    program_id,
                    from_file_id,
                    code_block.name,
                    code_block.kind.as_str(),
                    code_block.parent_section,
                    code_block_sequence(code_block, &file.code_blocks) as i64,
                    code_block.statement_count as i64,
                    code_block.start_offset as i64,
                    code_block.byte_len as i64,
                ],
            )?;
            if matches!(
                code_block.kind,
                crate::cobol::model::CodeBlockKind::Paragraph
            ) {
                increment_count(&mut paragraph_count_by_program, program_id);
            }
        }

        for external_op in &file.external_ops {
            let Some(program_id) = resolve_program_id(
                &program_ids,
                external_op.caller_name.as_deref(),
                default_program,
            ) else {
                continue;
            };
            tx.execute(
                "INSERT INTO external_ops(program_id, source_file_id, kind, verb, target, start_offset, byte_len) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    program_id,
                    from_file_id,
                    external_op.kind.as_str(),
                    external_op.verb,
                    external_op.target,
                    external_op.start_offset as i64,
                    external_op.byte_len as i64,
                ],
            )?;
            increment_count(&mut external_op_count_by_program, program_id);
        }

        for identifier in &file.identifiers {
            let Some(program_id) = resolve_program_id(
                &program_ids,
                identifier.caller_name.as_deref(),
                default_program,
            ) else {
                continue;
            };
            increment_count(&mut identifier_count_by_program, program_id);
            aggregate_mention(
                &mut identifier_mentions,
                program_id,
                from_file_id,
                &identifier.kind,
                &identifier.value,
                identifier.start_offset,
            );
        }

        for literal in &file.literals {
            let Some(program_id) = resolve_program_id(
                &program_ids,
                literal.caller_name.as_deref(),
                default_program,
            ) else {
                continue;
            };
            increment_count(&mut literal_count_by_program, program_id);
            aggregate_mention(
                &mut literal_mentions,
                program_id,
                from_file_id,
                &literal.kind,
                &literal.value,
                literal.start_offset,
            );
        }

        for call in &file.calls {
            match call.kind {
                CallKind::Static => static_calls += 1,
                CallKind::Dynamic => dynamic_calls += 1,
            }
            let caller_name = call.caller_name.as_deref().or(default_program);
            let Some(caller_name) = caller_name else {
                continue;
            };
            let Some(caller_program_id) = program_ids.get(caller_name).copied() else {
                continue;
            };
            tx.execute(
                "INSERT INTO call_edges(caller_program_id, callee_name, start_offset, byte_len, kind, using_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    caller_program_id,
                    call.target,
                    call.start_offset as i64,
                    call.byte_len as i64,
                    call.kind.as_str(),
                    call.using_count as i64,
                ],
            )?;

            increment_count(&mut outgoing_calls, caller_program_id);
            match call.kind {
                CallKind::Static => {
                    increment_count(&mut static_calls_by_program, caller_program_id)
                }
                CallKind::Dynamic => {
                    increment_count(&mut dynamic_calls_by_program, caller_program_id)
                }
            }

            if matches!(call.kind, CallKind::Static) {
                if let Some(callee_program_id) = program_ids.get(&call.target).copied() {
                    increment_count(&mut incoming_calls, callee_program_id);
                    referenced_by_files
                        .entry(callee_program_id)
                        .or_default()
                        .insert(from_file_id);
                }
            }

            if let Some(summary) = report_programs.get_mut(caller_name) {
                summary.calls.push(CallSummary {
                    target: call.target.clone(),
                    kind: call.kind,
                    using_count: call.using_count,
                });
            }
        }
    }

    for (key, aggregate) in identifier_mentions {
        tx.execute(
            "INSERT INTO identifiers(program_id, source_file_id, kind, value, occurrences, first_offset) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                key.program_id,
                key.source_file_id,
                key.kind,
                key.value,
                aggregate.occurrences as i64,
                aggregate.first_offset as i64,
            ],
        )?;
    }

    for (key, aggregate) in literal_mentions {
        tx.execute(
            "INSERT INTO literals(program_id, source_file_id, kind, value, occurrences, first_offset) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                key.program_id,
                key.source_file_id,
                key.kind,
                key.value,
                aggregate.occurrences as i64,
                aggregate.first_offset as i64,
            ],
        )?;
    }

    for file in files
        .iter()
        .filter(|f| f.file_type == CobolFileType::Copybook)
    {
        let Some(copybook_file_id) = file_ids.get(&file.path).copied() else {
            continue;
        };
        let copybook_name = file
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(clean_name)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| relative_path(root, &file.path).to_ascii_uppercase());
        let item_names = copybook_item_names
            .remove(&copybook_file_id)
            .unwrap_or_default();
        let contains_header_fields = item_names.iter().any(|name| looks_like_header_field(name));
        let contains_error_fields = item_names.iter().any(|name| looks_like_error_field(name));
        tx.execute(
            "INSERT INTO copybook_features(copybook_file_id, copybook_name, used_by_program_count, used_by_file_count, replacing_use_count, data_item_count, contains_header_fields, contains_error_fields) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                copybook_file_id,
                copybook_name,
                copybook_programs
                    .get(&copybook_file_id)
                    .map_or(0_i64, |ids| ids.len() as i64),
                copybook_files
                    .get(&copybook_file_id)
                    .map_or(0_i64, |ids| ids.len() as i64),
                copybook_replacing_counts
                    .get(&copybook_file_id)
                    .copied()
                    .unwrap_or(0) as i64,
                item_names.len() as i64,
                bool_to_int(contains_header_fields),
                bool_to_int(contains_error_fields),
            ],
        )?;
    }

    for (program_name, path) in &program_file {
        let Some(program_id) = program_ids.get(program_name).copied() else {
            continue;
        };
        let Some(source_file_id) = file_ids.get(path).copied() else {
            continue;
        };
        let incoming = incoming_calls.get(&program_id).copied().unwrap_or(0);
        let copybook_use_count = copybook_use_count_by_program
            .get(&program_id)
            .copied()
            .unwrap_or(0);
        tx.execute(
            "INSERT INTO program_features(program_id, source_file_id, incoming_call_count, outgoing_call_count, static_call_count, dynamic_call_count, copybook_use_count, distinct_copybook_count, referenced_by_file_count, is_entrypoint, has_heavy_copy_usage, data_item_count, paragraph_count, external_op_count, identifier_count, literal_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                program_id,
                source_file_id,
                incoming as i64,
                outgoing_calls.get(&program_id).copied().unwrap_or(0) as i64,
                static_calls_by_program.get(&program_id).copied().unwrap_or(0) as i64,
                dynamic_calls_by_program.get(&program_id).copied().unwrap_or(0) as i64,
                copybook_use_count as i64,
                distinct_copybooks_by_program
                    .get(&program_id)
                    .map_or(0_i64, |ids| ids.len() as i64),
                referenced_by_files
                    .get(&program_id)
                    .map_or(0_i64, |ids| ids.len() as i64),
                bool_to_int(incoming == 0),
                bool_to_int(copybook_use_count >= 3),
                data_item_count_by_program.get(&program_id).copied().unwrap_or(0) as i64,
                paragraph_count_by_program.get(&program_id).copied().unwrap_or(0) as i64,
                external_op_count_by_program.get(&program_id).copied().unwrap_or(0) as i64,
                identifier_count_by_program.get(&program_id).copied().unwrap_or(0) as i64,
                literal_count_by_program.get(&program_id).copied().unwrap_or(0) as i64,
            ],
        )?;
    }

    tx.commit()?;

    let mut programs = report_programs.into_values().collect::<Vec<_>>();
    programs.sort_by(|a, b| a.name.cmp(&b.name));
    unresolved_copybooks.sort();
    unresolved_copybooks.dedup();

    Ok(IndexReport {
        files,
        source_count,
        copybook_count,
        programs,
        copybook_uses,
        resolved_copybooks,
        unresolved_copybooks,
        static_calls,
        dynamic_calls,
        data_items,
    })
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn mtime_unix(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn increment_count<K>(map: &mut HashMap<K, usize>, key: K)
where
    K: Eq + Hash,
{
    *map.entry(key).or_default() += 1;
}

fn resolve_program_id(
    program_ids: &HashMap<String, i64>,
    explicit_name: Option<&str>,
    default_name: Option<&str>,
) -> Option<i64> {
    explicit_name
        .or(default_name)
        .and_then(|name| program_ids.get(name).copied())
}

fn aggregate_mention(
    map: &mut HashMap<MentionKey, MentionAggregate>,
    program_id: i64,
    source_file_id: i64,
    kind: &str,
    value: &str,
    start_offset: usize,
) {
    let key = MentionKey {
        program_id,
        source_file_id,
        kind: kind.to_string(),
        value: value.to_string(),
    };
    let entry = map.entry(key).or_insert(MentionAggregate {
        occurrences: 0,
        first_offset: start_offset,
    });
    entry.occurrences += 1;
    entry.first_offset = entry.first_offset.min(start_offset);
}

fn code_block_sequence(code_block: &ParsedCodeBlock, all_blocks: &[ParsedCodeBlock]) -> usize {
    all_blocks
        .iter()
        .position(|candidate| {
            candidate.start_offset == code_block.start_offset
                && candidate.name == code_block.name
                && candidate.kind == code_block.kind
        })
        .map_or(1, |idx| idx + 1)
}

fn looks_like_header_field(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.contains("HEADER") || upper.contains("HDR")
}

fn looks_like_error_field(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.contains("ERROR")
        || upper.contains("ERR")
        || upper.contains("SQLCODE")
        || upper.contains("RETURN-CODE")
        || upper.contains("RESP-CODE")
}

fn bool_to_int(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MentionKey {
    program_id: i64,
    source_file_id: i64,
    kind: String,
    value: String,
}

#[derive(Debug, Clone, Copy)]
struct MentionAggregate {
    occurrences: usize,
    first_offset: usize,
}
