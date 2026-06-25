use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    Static,
    Dynamic,
}

impl CallKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            CallKind::Static => "static",
            CallKind::Dynamic => "dynamic",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CallSummary {
    pub target: String,
    pub kind: CallKind,
    pub using_count: usize,
}

#[derive(Debug, Clone)]
pub struct CopybookSummary {
    pub name: String,
    pub resolved_path: Option<PathBuf>,
    pub has_replacing: bool,
}

#[derive(Debug, Clone)]
pub struct ProgramSummary {
    pub name: String,
    pub path: PathBuf,
    pub copybooks: Vec<CopybookSummary>,
    pub calls: Vec<CallSummary>,
    pub data_items: usize,
}

#[derive(Debug, Clone)]
pub struct IndexReport {
    pub files: Vec<crate::cobol::scanner::CobolFileEntry>,
    pub source_count: usize,
    pub copybook_count: usize,
    pub programs: Vec<ProgramSummary>,
    pub copybook_uses: usize,
    pub resolved_copybooks: usize,
    pub unresolved_copybooks: Vec<String>,
    pub static_calls: usize,
    pub dynamic_calls: usize,
    pub data_items: usize,
}

impl IndexReport {
    pub fn to_message(&self, db_path: &Path) -> String {
        let mut out = format!(
            "Project index initialized.\n  Files: {} (sources: {}, copybooks: {})\n  Programs: {}\n  COPY uses: {} resolved / {} total\n  CALL edges: {} static, {} dynamic\n  DATA items: {}\n  SQLite: {}",
            self.files.len(),
            self.source_count,
            self.copybook_count,
            self.programs.len(),
            self.resolved_copybooks,
            self.copybook_uses,
            self.static_calls,
            self.dynamic_calls,
            self.data_items,
            db_path.to_string_lossy(),
        );

        if !self.programs.is_empty() {
            out.push_str("\n\nPrograms:");
            for program in self.programs.iter().take(20) {
                out.push_str(&format!(
                    "\n  - {} ({})",
                    program.name,
                    program.path.to_string_lossy()
                ));
                if !program.copybooks.is_empty() {
                    let names = program
                        .copybooks
                        .iter()
                        .map(|c| {
                            if c.resolved_path.is_some() {
                                if c.has_replacing {
                                    format!("{}:resolved+replacing", c.name)
                                } else {
                                    format!("{}:resolved", c.name)
                                }
                            } else {
                                format!("{}:missing", c.name)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    out.push_str(&format!("\n      COPY: {}", names));
                }
                if !program.calls.is_empty() {
                    let calls = program
                        .calls
                        .iter()
                        .map(|c| {
                            format!("{}({}; USING {})", c.target, c.kind.as_str(), c.using_count)
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    out.push_str(&format!("\n      CALL: {}", calls));
                }
                if program.data_items > 0 {
                    out.push_str(&format!("\n      DATA: {} item(s)", program.data_items));
                }
            }
            if self.programs.len() > 20 {
                out.push_str(&format!(
                    "\n  ... {} more program(s)",
                    self.programs.len() - 20
                ));
            }
        }

        if !self.unresolved_copybooks.is_empty() {
            out.push_str("\n\nUnresolved COPY:");
            for name in self.unresolved_copybooks.iter().take(20) {
                out.push_str(&format!("\n  - {}", name));
            }
        }

        out
    }
}

#[derive(Debug)]
pub(crate) struct ParsedProgram {
    pub(crate) name: String,
    pub(crate) start_offset: usize,
    pub(crate) byte_len: usize,
}

#[derive(Debug)]
pub(crate) struct ParsedCopy {
    pub(crate) name: String,
    pub(crate) start_offset: usize,
    pub(crate) byte_len: usize,
    pub(crate) replacing_text: Option<String>,
}

#[derive(Debug)]
pub(crate) struct ParsedCall {
    pub(crate) caller_name: Option<String>,
    pub(crate) target: String,
    pub(crate) kind: CallKind,
    pub(crate) start_offset: usize,
    pub(crate) byte_len: usize,
    pub(crate) using_count: usize,
}

#[derive(Debug)]
pub(crate) struct ParsedDataItem {
    pub(crate) source_path: PathBuf,
    pub(crate) name: String,
    pub(crate) level: u16,
    pub(crate) parent_name: Option<String>,
    pub(crate) pic: Option<String>,
    pub(crate) usage_clause: Option<String>,
    pub(crate) occurs: Option<i64>,
    pub(crate) redefines: Option<String>,
    pub(crate) section: Option<String>,
    pub(crate) byte_offset: Option<i64>,
    pub(crate) byte_size: Option<i64>,
    pub(crate) storage_kind: Option<String>,
    pub(crate) layout_status: Option<String>,
    pub(crate) start_offset: usize,
    pub(crate) byte_len: usize,
}

#[derive(Debug)]
pub(crate) struct ParsedFile {
    pub(crate) path: PathBuf,
    pub(crate) programs: Vec<ParsedProgram>,
    pub(crate) copies: Vec<ParsedCopy>,
    pub(crate) calls: Vec<ParsedCall>,
}

#[derive(Debug)]
pub(crate) struct Token {
    pub(crate) text: String,
    pub(crate) start: usize,
    pub(crate) quoted: bool,
}

#[derive(Debug)]
pub(crate) struct LogicalLine {
    pub(crate) text: String,
    pub(crate) start_offset: usize,
    pub(crate) byte_len: usize,
}
