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
