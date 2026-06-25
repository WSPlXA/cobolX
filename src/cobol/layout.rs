use crate::cobol::model::ParsedDataItem;
use std::collections::HashMap;

#[derive(Debug)]
struct Frame {
    idx: usize,
    level: u16,
    base_offset: i64,
    cursor: i64,
}

#[derive(Debug, Clone, Copy)]
struct Storage {
    bytes: Option<i64>,
    kind: &'static str,
    status: &'static str,
}

pub(crate) fn compute_physical_layout(items: &mut [ParsedDataItem]) {
    let mut stack = vec![Frame {
        idx: usize::MAX,
        level: 0,
        base_offset: 0,
        cursor: 0,
    }];
    let mut known_offsets = HashMap::<String, i64>::with_capacity(items.len());

    for idx in 0..items.len() {
        let level = items[idx].level;
        while stack.last().is_some_and(|frame| frame.level >= level) {
            close_group(items, &mut stack, &mut known_offsets);
        }

        if matches!(level, 66 | 88) {
            items[idx].byte_offset = current_cursor(&stack);
            items[idx].byte_size = Some(0);
            items[idx].storage_kind = Some(
                if level == 66 {
                    "rename"
                } else {
                    "condition-name"
                }
                .to_string(),
            );
            items[idx].layout_status = Some("no-storage".to_string());
            known_offsets.insert(items[idx].name.clone(), items[idx].byte_offset.unwrap_or(0));
            continue;
        }

        let offset = items[idx]
            .redefines
            .as_ref()
            .and_then(|name| known_offsets.get(name).copied())
            .unwrap_or_else(|| current_cursor(&stack).unwrap_or(0));

        items[idx].byte_offset = Some(offset);

        let storage = classify_storage(&items[idx]);
        if storage.kind == "group" {
            items[idx].storage_kind = Some("group".to_string());
            items[idx].layout_status = Some("pending".to_string());
            stack.push(Frame {
                idx,
                level,
                base_offset: offset,
                cursor: offset,
            });
            known_offsets.insert(items[idx].name.clone(), offset);
            continue;
        }

        let occurs = item_occurs(&items[idx]);
        let total_bytes = storage.bytes.map(|bytes| bytes.saturating_mul(occurs));
        items[idx].byte_size = total_bytes;
        items[idx].storage_kind = Some(storage.kind.to_string());
        items[idx].layout_status = Some(storage.status.to_string());

        if let Some(total) = total_bytes {
            advance_parent(&mut stack, offset.saturating_add(total));
        }
        known_offsets.insert(items[idx].name.clone(), offset);
    }

    while stack.len() > 1 {
        close_group(items, &mut stack, &mut known_offsets);
    }
}

fn close_group(
    items: &mut [ParsedDataItem],
    stack: &mut Vec<Frame>,
    known_offsets: &mut HashMap<String, i64>,
) {
    let Some(frame) = stack.pop() else {
        return;
    };

    let unit_bytes = frame.cursor.saturating_sub(frame.base_offset);
    let total_bytes = unit_bytes.saturating_mul(item_occurs(&items[frame.idx]));
    items[frame.idx].byte_size = Some(total_bytes);
    items[frame.idx].layout_status = Some("derived".to_string());
    advance_parent(stack, frame.base_offset.saturating_add(total_bytes));
    known_offsets.insert(items[frame.idx].name.clone(), frame.base_offset);
}

fn current_cursor(stack: &[Frame]) -> Option<i64> {
    stack.last().map(|frame| frame.cursor)
}

fn advance_parent(stack: &mut [Frame], end_offset: i64) {
    if let Some(parent) = stack.last_mut() {
        parent.cursor = parent.cursor.max(end_offset);
    }
}

fn item_occurs(item: &ParsedDataItem) -> i64 {
    item.occurs.unwrap_or(1).max(1)
}

fn classify_storage(item: &ParsedDataItem) -> Storage {
    let usage = item
        .usage_clause
        .as_deref()
        .map(normalize_usage)
        .unwrap_or_else(|| "DISPLAY".to_string());

    if item.pic.is_none() {
        return match usage.as_str() {
            "COMP-1" | "COMPUTATIONAL-1" => Storage {
                bytes: Some(4),
                kind: "float",
                status: "exact",
            },
            "COMP-2" | "COMPUTATIONAL-2" => Storage {
                bytes: Some(8),
                kind: "float",
                status: "exact",
            },
            "POINTER" => Storage {
                bytes: Some(8),
                kind: "pointer",
                status: "estimated",
            },
            "INDEX" => Storage {
                bytes: Some(4),
                kind: "index",
                status: "estimated",
            },
            _ => Storage {
                bytes: None,
                kind: "group",
                status: "pending",
            },
        };
    }

    let pic = item.pic.as_deref().unwrap_or_default();
    match usage.as_str() {
        "COMP-3" | "COMPUTATIONAL-3" | "PACKED-DECIMAL" => {
            let digits = picture_digits(pic);
            Storage {
                bytes: digits.map(|d| (d + 2) / 2),
                kind: "packed-decimal",
                status: "exact",
            }
        }
        "BINARY" | "COMP" | "COMP-4" | "COMP-5" | "COMPUTATIONAL" | "COMPUTATIONAL-4"
        | "COMPUTATIONAL-5" => {
            let digits = picture_digits(pic);
            Storage {
                bytes: digits.and_then(binary_bytes_for_digits),
                kind: "binary",
                status: "estimated",
            }
        }
        "COMP-1" | "COMPUTATIONAL-1" => Storage {
            bytes: Some(4),
            kind: "float",
            status: "exact",
        },
        "COMP-2" | "COMPUTATIONAL-2" => Storage {
            bytes: Some(8),
            kind: "float",
            status: "exact",
        },
        "NATIONAL" => Storage {
            bytes: picture_positions(pic).map(|positions| positions.saturating_mul(2)),
            kind: "national",
            status: "estimated",
        },
        _ => {
            let positions = picture_positions(pic);
            let has_national = expanded_picture_chars(pic).any(|ch| ch == 'N');
            Storage {
                bytes: positions.map(|n| if has_national { n.saturating_mul(2) } else { n }),
                kind: "display",
                status: if has_national { "estimated" } else { "exact" },
            }
        }
    }
}

fn normalize_usage(usage: &str) -> String {
    usage
        .split_whitespace()
        .filter(|part| *part != "IS")
        .collect::<Vec<_>>()
        .join("-")
        .to_ascii_uppercase()
}

fn binary_bytes_for_digits(digits: i64) -> Option<i64> {
    match digits {
        1..=4 => Some(2),
        5..=9 => Some(4),
        10..=18 => Some(8),
        _ => None,
    }
}

fn picture_digits(pic: &str) -> Option<i64> {
    let digits = expanded_picture_chars(pic)
        .filter(|ch| matches!(*ch, '9' | 'Z' | '*'))
        .count() as i64;
    (digits > 0).then_some(digits)
}

fn picture_positions(pic: &str) -> Option<i64> {
    let positions = expanded_picture_chars(pic)
        .filter(|ch| !matches!(*ch, 'S' | 'V' | 'P'))
        .count() as i64;
    (positions > 0).then_some(positions)
}

fn expanded_picture_chars(pic: &str) -> impl Iterator<Item = char> + '_ {
    PictureChars {
        chars: pic.chars().peekable(),
        repeat_char: None,
        repeat_left: 0,
    }
}

struct PictureChars<I: Iterator<Item = char>> {
    chars: std::iter::Peekable<I>,
    repeat_char: Option<char>,
    repeat_left: usize,
}

impl<I: Iterator<Item = char>> Iterator for PictureChars<I> {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        if self.repeat_left > 0 {
            self.repeat_left -= 1;
            return self.repeat_char;
        }

        let ch = self.chars.next()?.to_ascii_uppercase();
        if self.chars.peek() == Some(&'(') {
            self.chars.next();
            let mut n = String::new();
            while let Some(next) = self.chars.peek().copied() {
                self.chars.next();
                if next == ')' {
                    break;
                }
                n.push(next);
            }
            let repeat = n.parse::<usize>().unwrap_or(1);
            if repeat > 1 {
                self.repeat_char = Some(ch);
                self.repeat_left = repeat - 1;
            }
        }

        Some(ch)
    }
}
