use std::fs::{self, File};
use std::io::Write;
use tempfile::tempdir;

#[test]
fn init_indexer_persists_program_copybook_and_call_edges() {
    let dir = tempdir().unwrap();
    let copy_dir = dir.path().join("copy");
    fs::create_dir_all(&copy_dir).unwrap();

    File::create(dir.path().join("MAIN.cbl"))
        .unwrap()
        .write_all(
            br#"
       IDENTIFICATION DIVISION.
       PROGRAM-ID. MAIN.
       DATA DIVISION.
       WORKING-STORAGE SECTION.
       01 WS-NEXT-PGM PIC X(8).
       COPY CUSTOMER.
       PROCEDURE DIVISION.
           CALL "SUB001"
               USING WS-NEXT-PGM.
           CALL WS-NEXT-PGM.
           STOP RUN.
"#,
        )
        .unwrap();

    File::create(dir.path().join("SUB001.cbl"))
        .unwrap()
        .write_all(
            br#"
       IDENTIFICATION DIVISION.
       PROGRAM-ID. SUB001.
       PROCEDURE DIVISION.
           EXIT PROGRAM.
"#,
        )
        .unwrap();

    File::create(dir.path().join("CUSTOMER.cpy"))
        .unwrap()
        .write_all(
            br#"
       01 CUSTOMER-REC.
          05 CUST-ID PIC X(10).
          05 CUST-BALANCE PIC S9(7)V99
             COMP-3.
          05 CUST-ALIAS REDEFINES CUST-ID PIC X(10).
          05 CUST-ADDR OCCURS
             3 TIMES
             PIC X(20).
"#,
        )
        .unwrap();

    let mut store = rdo::memory::MemoryStore::open_or_create(dir.path()).unwrap();
    let report = rdo::cobol::indexer::index_sandbox(dir.path(), &mut store).unwrap();

    assert_eq!(report.source_count, 2);
    assert_eq!(report.copybook_count, 1);
    assert_eq!(report.programs.len(), 2);
    assert_eq!(report.copybook_uses, 1);
    assert_eq!(report.resolved_copybooks, 1);
    assert_eq!(report.static_calls, 1);
    assert_eq!(report.dynamic_calls, 1);
    assert_eq!(report.data_items, 6);

    let conn = store.connection();
    let programs: i64 = conn
        .query_row("SELECT COUNT(*) FROM programs", [], |row| row.get(0))
        .unwrap();
    let copies: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM copybook_uses WHERE resolve_status = 'resolved'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let static_calls: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM call_edges WHERE kind = 'static'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let dynamic_calls: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM call_edges WHERE kind = 'dynamic'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let data_items: i64 = conn
        .query_row("SELECT COUNT(*) FROM data_items", [], |row| row.get(0))
        .unwrap();
    let cust_id_pic: String = conn
        .query_row(
            "SELECT pic FROM data_items WHERE name = 'CUST-ID'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let comp3_usage: String = conn
        .query_row(
            "SELECT usage_clause FROM data_items WHERE name = 'CUST-BALANCE'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let comp3_pic: String = conn
        .query_row(
            "SELECT pic FROM data_items WHERE name = 'CUST-BALANCE'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let redefines: String = conn
        .query_row(
            "SELECT redefines FROM data_items WHERE name = 'CUST-ALIAS'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let occurs: i64 = conn
        .query_row(
            "SELECT occurs FROM data_items WHERE name = 'CUST-ADDR'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let ws_layout: (i64, i64, String) = conn
        .query_row(
            "SELECT byte_offset, byte_size, storage_kind FROM data_items WHERE name = 'WS-NEXT-PGM'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let customer_group: (i64, i64, String) = conn
        .query_row(
            "SELECT byte_offset, byte_size, storage_kind FROM data_items WHERE name = 'CUSTOMER-REC'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let cust_id_layout: (i64, i64) = conn
        .query_row(
            "SELECT byte_offset, byte_size FROM data_items WHERE name = 'CUST-ID'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let comp3_layout: (i64, i64, String) = conn
        .query_row(
            "SELECT byte_offset, byte_size, storage_kind FROM data_items WHERE name = 'CUST-BALANCE'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let alias_layout: (i64, i64) = conn
        .query_row(
            "SELECT byte_offset, byte_size FROM data_items WHERE name = 'CUST-ALIAS'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let occurs_layout: (i64, i64) = conn
        .query_row(
            "SELECT byte_offset, byte_size FROM data_items WHERE name = 'CUST-ADDR'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    assert_eq!(programs, 2);
    assert_eq!(copies, 1);
    assert_eq!(static_calls, 1);
    assert_eq!(dynamic_calls, 1);
    assert_eq!(data_items, 6);
    assert_eq!(cust_id_pic, "X(10)");
    assert_eq!(comp3_pic, "S9(7)V99");
    assert_eq!(comp3_usage, "COMP-3");
    assert_eq!(redefines, "CUST-ID");
    assert_eq!(occurs, 3);
    assert_eq!(ws_layout, (0, 8, "display".to_string()));
    assert_eq!(customer_group, (8, 75, "group".to_string()));
    assert_eq!(cust_id_layout, (8, 10));
    assert_eq!(comp3_layout, (18, 5, "packed-decimal".to_string()));
    assert_eq!(alias_layout, (8, 10));
    assert_eq!(occurs_layout, (23, 60));
}

#[test]
fn init_indexer_persists_richer_semantic_index_data() {
    let dir = tempdir().unwrap();
    let copy_dir = dir.path().join("copy");
    fs::create_dir_all(&copy_dir).unwrap();

    File::create(dir.path().join("MAIN.cbl"))
        .unwrap()
        .write_all(
            br#"
       IDENTIFICATION DIVISION.
       PROGRAM-ID. MAIN.
       DATA DIVISION.
       WORKING-STORAGE SECTION.
       01 WS-NEXT-PGM PIC X(8).
       01 CUSTOMER-FILE PIC X(8).
       01 CUSTOMER-RECORD PIC X(80).
       COPY COMMONHDR.
       COPY ERRCODES.
       PROCEDURE DIVISION.
       MAIN-SECTION SECTION.
       MAIN-START.
           OPEN INPUT CUSTOMER-FILE.
           EXEC SQL
              SELECT CUST_ID
                INTO :WS-NEXT-PGM
                FROM CUSTOMER_TABLE
               WHERE STATUS = 'ER'
           END-EXEC.
           CALL "SUB001"
               USING WS-NEXT-PGM.
           CALL WS-NEXT-PGM.
           PERFORM HANDLE-ERROR.
           READ CUSTOMER-FILE.
       HANDLE-ERROR.
           EXEC CICS
               LINK PROGRAM('ERRHNDL')
               COMMAREA(WS-NEXT-PGM)
           END-EXEC.
           WRITE CUSTOMER-RECORD.
           CLOSE CUSTOMER-FILE.
           STOP RUN.
"#,
        )
        .unwrap();

    File::create(dir.path().join("SUB001.cbl"))
        .unwrap()
        .write_all(
            br#"
       IDENTIFICATION DIVISION.
       PROGRAM-ID. SUB001.
       PROCEDURE DIVISION.
           EXIT PROGRAM.
"#,
        )
        .unwrap();

    File::create(copy_dir.join("COMMONHDR.cpy"))
        .unwrap()
        .write_all(
            br#"
       01 COMMON-HDR.
          05 HDR-REQUEST-ID PIC X(16).
          05 HDR-DATE PIC X(8).
"#,
        )
        .unwrap();

    File::create(copy_dir.join("ERRCODES.cpy"))
        .unwrap()
        .write_all(
            br#"
       01 ERROR-INFO.
          05 ERR-CODE PIC X(4).
          05 ERR-MESSAGE PIC X(40).
"#,
        )
        .unwrap();

    let mut store = rdo::memory::MemoryStore::open_or_create(dir.path()).unwrap();
    rdo::cobol::indexer::index_sandbox(dir.path(), &mut store).unwrap();

    let conn = store.connection();

    let main_program_features: (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) = conn
        .query_row(
            "SELECT incoming_call_count, outgoing_call_count, static_call_count, dynamic_call_count, \
                    copybook_use_count, referenced_by_file_count, is_entrypoint, paragraph_count, \
                    external_op_count, literal_count \
             FROM program_features pf \
             JOIN programs p ON p.id = pf.program_id \
             WHERE p.name = 'MAIN'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                ))
            },
        )
        .unwrap();

    let sub_program_features: (i64, i64, i64) = conn
        .query_row(
            "SELECT incoming_call_count, referenced_by_file_count, is_entrypoint \
             FROM program_features pf \
             JOIN programs p ON p.id = pf.program_id \
             WHERE p.name = 'SUB001'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();

    let code_blocks: Vec<(String, String, i64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT cb.name, cb.kind, cb.statement_count \
                 FROM code_blocks cb \
                 JOIN programs p ON p.id = cb.program_id \
                 WHERE p.name = 'MAIN' \
                 ORDER BY sequence_no",
            )
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };

    let external_ops: Vec<(String, String, Option<String>)> = {
        let mut stmt = conn
            .prepare(
                "SELECT kind, verb, target \
                 FROM external_ops eo \
                 JOIN programs p ON p.id = eo.program_id \
                 WHERE p.name = 'MAIN' \
                 ORDER BY eo.id",
            )
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };

    let identifiers: Vec<(String, String, i64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT kind, value, occurrences \
                 FROM identifiers i \
                 JOIN programs p ON p.id = i.program_id \
                 WHERE p.name = 'MAIN' \
                 ORDER BY kind, value",
            )
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };

    let literals: Vec<(String, String, i64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT kind, value, occurrences \
                 FROM literals l \
                 JOIN programs p ON p.id = l.program_id \
                 WHERE p.name = 'MAIN' \
                 ORDER BY kind, value",
            )
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };

    let copybook_features: Vec<(String, i64, i64, i64, i64, i64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT cf.copybook_name, cf.used_by_program_count, cf.used_by_file_count, \
                        cf.data_item_count, cf.contains_header_fields, cf.contains_error_fields \
                 FROM copybook_features cf \
                 ORDER BY cf.copybook_name",
            )
            .unwrap();
        stmt.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    };

    assert_eq!(main_program_features, (0, 2, 1, 1, 2, 0, 1, 2, 8, 3));
    assert_eq!(sub_program_features, (1, 1, 0));
    assert_eq!(
        code_blocks,
        vec![
            ("MAIN-SECTION".to_string(), "section".to_string(), 0),
            ("MAIN-START".to_string(), "paragraph".to_string(), 6),
            ("HANDLE-ERROR".to_string(), "paragraph".to_string(), 4),
        ]
    );
    assert!(external_ops.contains(&(
        "file_io".to_string(),
        "OPEN".to_string(),
        Some("CUSTOMER-FILE".to_string())
    )));
    assert!(external_ops.contains(&(
        "exec_sql".to_string(),
        "SELECT".to_string(),
        Some("CUSTOMER_TABLE".to_string())
    )));
    assert!(external_ops.contains(&(
        "call_literal".to_string(),
        "CALL".to_string(),
        Some("SUB001".to_string())
    )));
    assert!(external_ops.contains(&(
        "call_identifier".to_string(),
        "CALL".to_string(),
        Some("WS-NEXT-PGM".to_string())
    )));
    assert!(external_ops.contains(&(
        "exec_cics".to_string(),
        "LINK".to_string(),
        Some("ERRHNDL".to_string())
    )));
    assert!(identifiers.contains(&(
        "sql_table".to_string(),
        "CUSTOMER_TABLE".to_string(),
        1
    )));
    assert!(identifiers.contains(&(
        "file_name".to_string(),
        "CUSTOMER-FILE".to_string(),
        3
    )));
    assert!(identifiers.contains(&(
        "paragraph_name".to_string(),
        "HANDLE-ERROR".to_string(),
        1
    )));
    assert!(identifiers.contains(&(
        "data_name".to_string(),
        "ERR-CODE".to_string(),
        1
    )));
    assert!(literals.contains(&(
        "call_target".to_string(),
        "SUB001".to_string(),
        1
    )));
    assert!(literals.contains(&(
        "exec_cics_program".to_string(),
        "ERRHNDL".to_string(),
        1
    )));
    assert!(literals.contains(&(
        "string_literal".to_string(),
        "ER".to_string(),
        1
    )));
    assert_eq!(
        copybook_features,
        vec![
            ("COMMONHDR".to_string(), 1, 1, 3, 1, 0),
            ("ERRCODES".to_string(), 1, 1, 3, 0, 1),
        ]
    );
}
