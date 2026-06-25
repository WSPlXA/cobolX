use crate::cobol::copybook::{build_copybook_index, resolve_copybook};
use crate::cobol::data_parser::collect_data_items;
use crate::cobol::layout::compute_physical_layout;
pub use crate::cobol::model::{
    CallKind, CallSummary, CopybookSummary, IndexReport, ProgramSummary,
};
use crate::cobol::scanner::{CobolFileType, scan_sandbox};
use crate::cobol::source_parser::parse_source_file;
use crate::memory::MemoryStore;
use rusqlite::params;
use std::collections::HashMap;
use std::error::Error;
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

            if let Some(program_name) = default_program {
                if let Some(summary) = report_programs.get_mut(program_name) {
                    summary.copybooks.push(CopybookSummary {
                        name: copy.name.clone(),
                        resolved_path: resolved,
                        has_replacing: copy.replacing_text.is_some(),
                    });
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
                tx.execute(
                    "INSERT INTO data_items(program_id, source_file_id, name, level, parent_name, pic, usage_clause, occurs, redefines, section, byte_offset, byte_size, storage_kind, layout_status, start_offset, byte_len) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                    params![
                        program_id,
                        file_ids.get(&item.source_path).copied(),
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
                if let Some(summary) = report_programs.get_mut(program_name) {
                    summary.data_items += 1;
                }
            }
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

            if let Some(summary) = report_programs.get_mut(caller_name) {
                summary.calls.push(CallSummary {
                    target: call.target.clone(),
                    kind: call.kind,
                    using_count: call.using_count,
                });
            }
        }
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
