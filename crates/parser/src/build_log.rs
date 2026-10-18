use std::{cmp::Ordering, path::PathBuf};

use once_cell::sync::Lazy;
use regex::Regex;
use syntax::{BuildError, BuildErrorLevel, BuildLog};

const MAX_LINE_LENGTH: usize = 79;

static PACKAGE_MESSAGE_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new("^\\([a-zA-Z_\\-]+\\)\\s*(?P<msg>.*)$").unwrap());

// Keep error blocks separate: an error without `l.N` must not borrow the
// location of a later error. Explicit file:line locations take precedence over
// the ambiguous parenthesis-based file stack in traditional TeX logs.
static TEX_ERROR_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?m)^(?:! (?P<msg>[^\r\n]*)|(?P<file>[^\r\n]+?\.[A-Za-z0-9_-]+"?):(?P<line>\d+): (?P<file_msg>[^\r\n]*))"#,
    )
    .unwrap()
});

static CONTEXT_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?m)^l\.(?P<line>\d+)(?: (?P<hint>[^\r\n]*))?\r?$").unwrap());

static WARNING_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new("(?P<msg>(LaTeX|Package [a-zA-Z_\\-]+) Warning: [^\r\n]*?(on input line (?P<line>\\d+))?\\.)[\r\n]").unwrap()
});

static BAD_BOX_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new("(?P<msg>(Ov|Und)erfull \\\\[hv]box[^\r\n]*lines? (?P<line>\\d+)[^\r\n]*)").unwrap()
});

pub fn parse_build_log(log: &str) -> BuildLog {
    let log = prepare_log(log);
    let mut ranges: Vec<FileRange> = log
        .match_indices('(')
        .filter_map(|(start, _)| FileRange::create(&log, start))
        .collect();
    ranges.sort();

    let tex_errors = extract_errors(&log, &ranges);
    let warnings = extract_matches(&log, &ranges, &WARNING_REGEX, BuildErrorLevel::Warning);
    let bad_boxes = extract_matches(&log, &ranges, &BAD_BOX_REGEX, BuildErrorLevel::Warning);

    let errors = [tex_errors, warnings, bad_boxes].concat();
    BuildLog { errors }
}

fn parse_line(value: &str) -> Option<u32> {
    value.parse::<u32>().ok()?.checked_sub(1)
}

fn extract_errors(log: &str, ranges: &[FileRange]) -> Vec<BuildError> {
    let headers: Vec<_> = TEX_ERROR_REGEX
        .captures_iter(log)
        .filter(|header| !CONTEXT_REGEX.is_match(&header[0]))
        .collect();
    let mut errors = Vec::new();
    for (index, header) in headers.iter().enumerate() {
        let start = header.get(0).unwrap();
        let end = headers
            .get(index + 1)
            .map_or(log.len(), |next| next.get(0).unwrap().start());
        let context = CONTEXT_REGEX.captures(&log[start.end()..end]);
        let context_line = context.as_ref().and_then(|c| parse_line(&c["line"]));
        let explicit = header.name("file");
        let line = if explicit.is_some() {
            header.name("line").and_then(|m| parse_line(m.as_str()))
        } else {
            context_line
        };
        let hint = context
            .filter(|_| line.is_some() && line == context_line)
            .and_then(|c| c.name("hint"))
            .map(|m| {
                m.as_str()
                    .trim()
                    .strip_prefix("...")
                    .unwrap_or(m.as_str().trim())
                    .to_owned()
            });
        let path = explicit
            .map(|m| PathBuf::from(m.as_str().trim_matches('"')))
            .or_else(|| {
                ranges
                    .iter()
                    .find(|r| r.contains(start.start()))
                    .map(|r| r.path.clone())
            });
        if let Some(relative_path) = path {
            errors.push(BuildError {
                relative_path,
                level: BuildErrorLevel::Error,
                message: header
                    .name("msg")
                    .or_else(|| header.name("file_msg"))
                    .unwrap()
                    .as_str()
                    .to_owned(),
                line,
                hint,
            });
        }
    }
    errors
}

fn extract_matches(
    log: &str,
    ranges: &[FileRange],
    regex: &Regex,
    level: BuildErrorLevel,
) -> Vec<BuildError> {
    let mut errors = Vec::new();
    for result in regex.find_iter(log) {
        let captures = regex.captures(&log[result.start()..result.end()]).unwrap();
        let message = captures
            .name("msg")
            .or_else(|| captures.name("msg1"))
            .or_else(|| captures.name("msg2"))
            .unwrap()
            .as_str()
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned();

        if let Some(range) = ranges.iter().find(|range| range.contains(result.start())) {
            let line = captures
                .name("line")
                .and_then(|result| parse_line(result.as_str()));

            let hint: Option<String> = if line.is_some() {
                captures
                    .name("hint")
                    .map(|r| String::from(r.as_str().trim()))
            } else {
                None
            };

            errors.push(BuildError {
                relative_path: range.path.clone(),
                level,
                message,
                line,
                hint,
            });
        }
    }
    errors
}

fn prepare_log(log: &str) -> String {
    let mut old_lines = log.lines().peekable();
    let mut new_lines: Vec<String> = Vec::new();
    while let Some(line) = old_lines.next() {
        if PACKAGE_MESSAGE_REGEX.is_match(line) {
            let captures = PACKAGE_MESSAGE_REGEX.captures(line).unwrap();
            if let Some(last_line) = new_lines.last_mut() {
                last_line.push(' ');
                last_line.push_str(captures.name("msg").unwrap().as_str());
            }
        } else if line.chars().count() == MAX_LINE_LENGTH {
            let mut new_line = String::new();
            new_line.push_str(line);
            while let Some(&old_line) = old_lines.peek() {
                if TEX_ERROR_REGEX.is_match(old_line) || CONTEXT_REGEX.is_match(old_line) {
                    break;
                }
                old_lines.next();
                new_line.push_str(old_line);
                if old_line.chars().count() != MAX_LINE_LENGTH {
                    break;
                }
            }
            new_lines.push(new_line);
        } else {
            new_lines.push(line.to_owned());
        }
    }
    new_lines.join("\n")
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct FileRange {
    pub path: PathBuf,
    pub start: usize,
    pub end: usize,
}

impl FileRange {
    fn create(log: &str, start: usize) -> Option<Self> {
        let (path, path_end) = file_name(log, start + 1)?;
        let mut balance = 1;
        let mut end = path_end;
        // Parentheses belonging to the filename are not file-stack markers.
        for c in log[path_end..].chars() {
            if balance <= 0 {
                break;
            }
            match c {
                '(' => balance += 1,
                ')' => balance -= 1,
                _ => {}
            }
            end += c.len_utf8();
        }
        Some(Self { path, start, end })
    }

    fn len(&self) -> usize {
        self.end - self.start + 1
    }

    fn contains(&self, index: usize) -> bool {
        index >= self.start && index <= self.end
    }
}

fn file_name(log: &str, start: usize) -> Option<(PathBuf, usize)> {
    let line = log[start..].split(['\r', '\n']).next()?;
    let supported = |path: &str| {
        path.rsplit_once('.').is_some_and(|(_, extension)| {
            [
                "tex", "sty", "cls", "def", "ltx", "clo", "cfg", "fd", "lco", "rnw",
            ]
            .iter()
            .any(|ext| extension.eq_ignore_ascii_case(ext))
        })
    };
    if let Some(quoted) = line.strip_prefix('"') {
        let end = quoted.find('"')?;
        let path = &quoted[..end];
        return supported(path).then(|| (path.into(), start + end + 2));
    }

    let mut balance = 0;
    let mut chars = line.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        match c {
            '(' => balance += 1,
            ')' if balance == 0 => return None,
            ')' => balance -= 1,
            _ => {}
        }
        let end = index + c.len_utf8();
        let boundary = chars
            .peek()
            .is_none_or(|(_, next)| next.is_whitespace() || matches!(next, ')' | '('));
        if balance == 0 && boundary && supported(&line[..end]) {
            return Some((line[..end].into(), start + end));
        }
    }
    None
}

impl Ord for FileRange {
    fn cmp(&self, other: &Self) -> Ordering {
        self.len().cmp(&other.len())
    }
}

impl PartialOrd for FileRange {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests;
