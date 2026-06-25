use crate::cobol::model::{LogicalLine, Token};

pub(crate) fn logical_lines(content: &str) -> Vec<LogicalLine> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_start = 0usize;
    let mut current_len = 0usize;
    let mut offset = 0usize;

    for raw_line in content.lines() {
        let raw_start = offset;
        let raw_len = raw_line.len();
        offset += raw_len + 1;

        let Some(code) = code_line(raw_line) else {
            continue;
        };
        let trimmed = code.trim();
        if trimmed.is_empty() {
            continue;
        }

        if current.is_empty() {
            current_start = raw_start + code.find(trimmed).unwrap_or(0);
            current_len = raw_len;
        } else {
            current.push(' ');
            current_len = (raw_start + raw_len).saturating_sub(current_start);
        }
        current.push_str(trimmed);

        if has_statement_terminator(trimmed) {
            lines.push(LogicalLine {
                text: std::mem::take(&mut current),
                start_offset: current_start,
                byte_len: current_len,
            });
            current_len = 0;
        }
    }

    if !current.is_empty() {
        lines.push(LogicalLine {
            text: current,
            start_offset: current_start,
            byte_len: current_len,
        });
    }

    lines
}

pub(crate) fn tokenize(line: &str) -> Vec<Token> {
    let bytes = line.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\'' || b == b'"' {
            let quote = b;
            let start = i;
            i += 1;
            let text_start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            tokens.push(Token {
                text: line[text_start..i].to_ascii_uppercase(),
                start,
                quoted: true,
            });
            if i < bytes.len() {
                i += 1;
            }
        } else if is_name_byte(b) {
            let start = i;
            while i < bytes.len() && is_name_byte(bytes[i]) {
                i += 1;
            }
            tokens.push(Token {
                text: line[start..i].to_ascii_uppercase(),
                start,
                quoted: false,
            });
        } else {
            i += 1;
        }
    }

    tokens
}

pub(crate) fn clean_name(name: &str) -> String {
    name.trim_matches('.')
        .trim_matches(',')
        .trim()
        .to_ascii_uppercase()
}

fn code_line(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('*') {
        return None;
    }
    if matches!(line.as_bytes().get(6), Some(b'*' | b'/')) {
        return None;
    }
    line.split("*>").next().map(str::trim_end)
}

fn has_statement_terminator(line: &str) -> bool {
    let mut quote = None::<u8>;
    let bytes = line.as_bytes();

    for (idx, b) in bytes.iter().enumerate() {
        match (quote, *b) {
            (Some(q), c) if c == q => quote = None,
            (None, b'\'' | b'"') => quote = Some(*b),
            (None, b'.') => {
                let next = bytes.get(idx + 1).copied();
                if next.map_or(true, |c| c.is_ascii_whitespace()) {
                    return true;
                }
            }
            _ => {}
        }
    }

    false
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'$' | b'#' | b'@')
}
