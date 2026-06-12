use std::ops::Range;
use std::path::Path;

pub fn split_parse_span(message: &str) -> (String, Option<Range<usize>>) {
    let Some((body, raw_span)) = message.rsplit_once(" @") else {
        return (message.to_string(), None);
    };
    let Some((start, end)) = raw_span.split_once("..") else {
        return (message.to_string(), None);
    };
    let Ok(start) = start.parse::<usize>() else {
        return (message.to_string(), None);
    };
    let Ok(end) = end.parse::<usize>() else {
        return (message.to_string(), None);
    };
    (body.to_string(), Some(start..end))
}

pub fn print(path: &Path, source: &str, span: Range<usize>, level: &str, message: &str) {
    // First message line goes in the machine-readable header (the VS Code
    // extension parses this exact format); extra lines (help: ...) go below.
    let mut lines = message.lines();
    let first = lines.next().unwrap_or(message);

    let (line, col) = line_col(source, span.start);
    let (end_line, end_col) = line_col(source, span.end.max(span.start + 1));
    eprintln!(
        "{}:{}:{}:{}:{}: {}: {}",
        path.display(),
        line,
        col,
        end_line,
        end_col,
        level,
        first
    );

    print_snippet(source, &span, line, col);

    for extra in lines {
        eprintln!("  {}", extra.trim_start());
    }
}

// Shows the offending source line with a caret underline:
//      12 |     if shared.n > 0 {
//         |                 ^
fn print_snippet(source: &str, span: &Range<usize>, line: usize, col: usize) {
    if span.start >= source.len() && source.is_empty() {
        return;
    }
    let start = span.start.min(source.len());
    let line_start = source[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = source[line_start..]
        .find('\n')
        .map(|i| line_start + i)
        .unwrap_or(source.len());
    let text = &source[line_start..line_end];
    if text.trim().is_empty() {
        return;
    }

    let span_end = span.end.min(line_end).max(start);
    // .get() devuelve None si el corte cae en medio de un char UTF-8
    let width = source
        .get(start..span_end)
        .map(|s| s.chars().count())
        .unwrap_or(1)
        .max(1);

    let gutter = format!("{:>5} ", line);
    eprintln!("{}| {}", gutter, text);
    eprintln!("{}| {}{}", " ".repeat(gutter.len()), " ".repeat(col.saturating_sub(1)), "^".repeat(width));
}

fn line_col(source: &str, byte_pos: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    let target = byte_pos.min(source.len());

    for (idx, ch) in source.char_indices() {
        if idx >= target {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }

    (line, col)
}
