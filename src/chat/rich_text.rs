//! Định dạng chữ tự động khi gửi qua API/automation (markdown nhẹ, tự nhấn mạnh số kèm đơn vị, kiểm tra ai_markup).
use std::cmp::Reverse;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{json, Value};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

use crate::contract::GwError;

pub(super) const MAX_TEXT_UTF16: usize = 4096;
pub(super) const MAX_ENTITIES: usize = 100;
/// Trần ĐẦU VÀO THÔ (kể cả dấu định dạng) kiểm TRƯỚC khi phân tích: dấu markdown chỉ bị bỏ đi nên chữ sau khi
/// dựng không bao giờ dài hơn đầu vào; cho phép gấp 4 lần trần chữ để chứa dấu định dạng. Phân tích có chỗ bậc hai,
/// nên kiểm kích thước ở biên chặn chi phí CPU thay vì chờ phân tích xong mới từ chối.
pub(super) const MAX_RAW_UTF16: usize = MAX_TEXT_UTF16 * 4;
const MAX_URL_BYTES: usize = 2048;
const MAX_AI_INSERTED_EMOJIS: usize = 16;
const MAX_INLINE_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum FormatMode {
    #[default]
    Plain,
    Markdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct RichOptions<'a> {
    pub format: FormatMode,
    pub auto_emphasis: bool,
    pub ai_markup: Option<&'a str>,
}

impl RichOptions<'_> {
    pub(super) fn is_active(&self) -> bool {
        self.format == FormatMode::Markdown || self.auto_emphasis || self.ai_markup.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum EntityKind {
    BlockQuote,
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Spoiler,
    Code,
    Pre(Option<String>),
    TextUrl(String),
}

impl EntityKind {
    fn rank(&self) -> u8 {
        match self {
            Self::BlockQuote => 0,
            Self::Pre(_) => 1,
            Self::Code => 2,
            Self::TextUrl(_) => 3,
            Self::Bold => 4,
            Self::Italic => 5,
            Self::Underline => 6,
            Self::Strikethrough => 7,
            Self::Spoiler => 8,
        }
    }

    fn extra_key(&self) -> &str {
        match self {
            Self::Pre(Some(lang)) => lang.as_str(),
            Self::TextUrl(url) => url.as_str(),
            _ => "",
        }
    }

    pub(super) fn to_tdlib_type(&self) -> Value {
        match self {
            Self::BlockQuote => json!({"@type":"textEntityTypeBlockQuote"}),
            Self::Bold => json!({"@type":"textEntityTypeBold"}),
            Self::Italic => json!({"@type":"textEntityTypeItalic"}),
            Self::Underline => json!({"@type":"textEntityTypeUnderline"}),
            Self::Strikethrough => json!({"@type":"textEntityTypeStrikethrough"}),
            Self::Spoiler => json!({"@type":"textEntityTypeSpoiler"}),
            Self::Code => json!({"@type":"textEntityTypeCode"}),
            Self::Pre(None) => json!({"@type":"textEntityTypePre"}),
            Self::Pre(Some(lang)) => json!({"@type":"textEntityTypePreCode","language":lang}),
            Self::TextUrl(url) => json!({"@type":"textEntityTypeTextUrl","url":url}),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RichEntity {
    pub offset: usize,
    pub length: usize,
    pub kind: EntityKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RichText {
    pub text: String,
    pub entities: Vec<RichEntity>,
}

pub(super) fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

pub(super) fn is_utf16_boundary(units: &[u16], pos: usize) -> bool {
    pos <= units.len() && (pos == units.len() || !(0xDC00..=0xDFFF).contains(&units[pos]))
}

pub(super) fn is_allowed_url(raw: &str) -> bool {
    if raw.is_empty() || raw.len() > MAX_URL_BYTES {
        return false;
    }
    if raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return false;
    }
    let rest = if raw.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://")) {
        &raw[8..]
    } else if raw.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("http://")) {
        &raw[7..]
    } else if raw.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("tg://")) {
        &raw[5..]
    } else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    !authority.is_empty() && !authority.starts_with('.') && !authority.contains('@')
}

pub(super) fn has_rich_params(body: &Value) -> bool {
    body.get("format").is_some_and(|v| !v.is_null())
        || body.get("auto_emphasis").is_some_and(|v| !v.is_null())
        || body.get("ai_markup").is_some_and(|v| !v.is_null())
}

pub(super) fn parse_options(body: &Value) -> Result<RichOptions<'_>, GwError> {
    let format = match body.get("format") {
        None | Some(Value::Null) => FormatMode::Plain,
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "plain" => FormatMode::Plain,
            "markdown" => FormatMode::Markdown,
            _ => return Err(GwError::bad_request("format phải là plain hoặc markdown.")),
        },
        Some(_) => return Err(GwError::bad_request("format phải là chuỗi (plain hoặc markdown).")),
    };
    let auto_emphasis = match body.get("auto_emphasis") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(GwError::bad_request("auto_emphasis phải là boolean (true hoặc false).")),
    };
    let ai_markup = match body.get("ai_markup") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.as_str()),
        Some(_) => return Err(GwError::bad_request("ai_markup phải là chuỗi.")),
    };
    Ok(RichOptions { format, auto_emphasis, ai_markup })
}

fn is_escapable(c: char) -> bool {
    matches!(c, '\\' | '*' | '_' | '~' | '`' | '|' | '[' | ']' | '(' | ')' | '>')
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || is_combining_mark(c)
}

fn find_inline_code_end(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let run = bytes[i..].iter().take_while(|&&b| b == b'`').count();
            if run == 1 && i > 0 {
                return Some(i);
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

fn split_fenced_code(inner: &str) -> Option<(Option<String>, &str)> {
    if let Some(nl) = inner.find('\n') {
        let first = inner[..nl].strip_suffix('\r').unwrap_or(&inner[..nl]);
        let rest = &inner[nl + 1..];
        let cand = first.trim();
        let (lang, raw_body) = if !cand.is_empty()
            && cand.chars().count() <= 64
            && !cand.chars().any(|c| c.is_whitespace() || c.is_control() || c == '`')
        {
            (Some(cand.to_owned()), rest)
        } else if cand.is_empty() {
            (None, rest)
        } else {
            (None, inner)
        };
        let body = raw_body
            .strip_suffix("\r\n")
            .or_else(|| raw_body.strip_suffix('\n'))
            .unwrap_or(raw_body);
        if body.is_empty() {
            None
        } else {
            Some((lang, body))
        }
    } else {
        let body = inner.trim();
        if body.is_empty() {
            None
        } else {
            Some((None, body))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DelimMarker {
    BoldStar,
    ItalicStar,
    Underline,
    ItalicUnder,
    Strike,
    Spoiler,
}

impl DelimMarker {
    fn raw(self) -> &'static str {
        match self {
            Self::BoldStar => "**",
            Self::ItalicStar => "*",
            Self::Underline => "__",
            Self::ItalicUnder => "_",
            Self::Strike => "~~",
            Self::Spoiler => "||",
        }
    }

    fn kind(self) -> EntityKind {
        match self {
            Self::BoldStar => EntityKind::Bold,
            Self::ItalicStar | Self::ItalicUnder => EntityKind::Italic,
            Self::Underline => EntityKind::Underline,
            Self::Strike => EntityKind::Strikethrough,
            Self::Spoiler => EntityKind::Spoiler,
        }
    }
}

enum Token {
    Literal {
        text: String,
        extra_entities: Vec<RichEntity>,
    },
    Delim {
        marker: DelimMarker,
        can_open: bool,
        can_close: bool,
        matched: bool,
    },
}

fn push_literal_char(tokens: &mut Vec<Token>, ch: char) {
    if let Some(Token::Literal { text, extra_entities }) = tokens.last_mut() {
        if extra_entities.is_empty() {
            text.push(ch);
            return;
        }
    }
    let mut text = String::new();
    text.push(ch);
    tokens.push(Token::Literal { text, extra_entities: Vec::new() });
}

fn find_link_label_end(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    let mut depth = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                if let Some(next) = s[i + 1..].chars().next() {
                    i += 1 + next.len_utf8();
                } else {
                    i += 1;
                }
            }
            b'`' => {
                let run = bytes[i..].iter().take_while(|&&b| b == b'`').count();
                if run == 1 {
                    if let Some(rel) = find_inline_code_end(&s[i + 1..]) {
                        i = i + 1 + rel + 1;
                        continue;
                    }
                }
                i += run;
            }
            b'[' => {
                depth += 1;
                i += 1;
            }
            b']' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
                i += 1;
            }
            b'\n' | b'\r' => return None,
            _ => {
                let ch = s[i..].chars().next().unwrap();
                i += ch.len_utf8();
            }
        }
    }
    None
}

fn find_link_url_end(s: &str) -> Option<usize> {
    let mut paren_depth = 1usize;
    for (idx, ch) in s.char_indices() {
        match ch {
            '(' => paren_depth += 1,
            ')' => {
                paren_depth -= 1;
                if paren_depth == 0 {
                    return Some(idx);
                }
            }
            '\n' | '\r' => return None,
            _ => {}
        }
    }
    None
}

fn try_parse_link(src: &str, pos: usize, depth: usize) -> Option<(usize, Token)> {
    if depth >= MAX_INLINE_DEPTH || !src[pos..].starts_with('[') {
        return None;
    }
    let label_start = pos + 1;
    let label_rel_end = find_link_label_end(&src[label_start..])?;
    let label_end = label_start + label_rel_end;
    if !src[label_end + 1..].starts_with('(') {
        return None;
    }
    let url_start = label_end + 2;
    let url_rel_end = find_link_url_end(&src[url_start..])?;
    let url_end = url_start + url_rel_end;
    let url = &src[url_start..url_end];
    if !is_allowed_url(url) {
        return None;
    }
    let label_raw = &src[label_start..label_end];
    let label_rich = parse_inline_depth(label_raw, depth + 1);
    if label_rich.text.is_empty() {
        return None;
    }
    let mut extra_entities = label_rich.entities;
    extra_entities.push(RichEntity {
        offset: 0,
        length: utf16_len(&label_rich.text),
        kind: EntityKind::TextUrl(url.to_owned()),
    });
    Some((
        url_end + 1,
        Token::Literal {
            text: label_rich.text,
            extra_entities,
        },
    ))
}

fn has_nonempty_literal(slice: &[Token]) -> bool {
    slice.iter().any(|t| matches!(t, Token::Literal { text, .. } if !text.is_empty()))
}

fn parse_inline(src: &str) -> RichText {
    parse_inline_depth(src, 0)
}

fn parse_inline_depth(src: &str, depth: usize) -> RichText {
    let bytes = src.as_bytes();
    let mut tokens: Vec<Token> = Vec::new();
    let mut pos = 0usize;

    while pos < bytes.len() {
        match bytes[pos] {
            b'\\' => {
                if let Some(next) = src[pos + 1..].chars().next() {
                    if is_escapable(next) {
                        push_literal_char(&mut tokens, next);
                        pos += 1 + next.len_utf8();
                        continue;
                    }
                }
                push_literal_char(&mut tokens, '\\');
                pos += 1;
            }
            b'`' => {
                let run = bytes[pos..].iter().take_while(|&&b| b == b'`').count();
                if run == 1 {
                    if let Some(rel) = find_inline_code_end(&src[pos + 1..]) {
                        let code = &src[pos + 1..pos + 1 + rel];
                        let len_u16 = utf16_len(code);
                        tokens.push(Token::Literal {
                            text: code.to_owned(),
                            extra_entities: vec![RichEntity {
                                offset: 0,
                                length: len_u16,
                                kind: EntityKind::Code,
                            }],
                        });
                        pos = pos + 1 + rel + 1;
                        continue;
                    }
                }
                for _ in 0..run {
                    push_literal_char(&mut tokens, '`');
                }
                pos += run;
            }
            b'[' => {
                if let Some((next_pos, link_tok)) = try_parse_link(src, pos, depth) {
                    tokens.push(link_tok);
                    pos = next_pos;
                } else {
                    push_literal_char(&mut tokens, '[');
                    pos += 1;
                }
            }
            b @ (b'*' | b'_') => {
                let run = bytes[pos..].iter().take_while(|&&x| x == b).count();
                let run_end = pos + run;
                let prev = src[..pos].chars().next_back();
                let next = src[run_end..].chars().next();
                let prev_ws = prev.map_or(true, char::is_whitespace);
                let next_ws = next.map_or(true, char::is_whitespace);
                let (can_open, can_close) = if b == b'_' {
                    (
                        !next_ws && !prev.is_some_and(is_word_char),
                        !prev_ws && !next.is_some_and(is_word_char),
                    )
                } else {
                    (!next_ws, !prev_ws)
                };
                let (double_m, single_m) = if b == b'*' {
                    (DelimMarker::BoldStar, DelimMarker::ItalicStar)
                } else {
                    (DelimMarker::Underline, DelimMarker::ItalicUnder)
                };
                let mut rem = run;
                while rem >= 2 {
                    tokens.push(Token::Delim {
                        marker: double_m,
                        can_open,
                        can_close,
                        matched: false,
                    });
                    rem -= 2;
                }
                if rem == 1 {
                    tokens.push(Token::Delim {
                        marker: single_m,
                        can_open,
                        can_close,
                        matched: false,
                    });
                }
                pos = run_end;
            }
            b @ (b'~' | b'|') => {
                let run = bytes[pos..].iter().take_while(|&&x| x == b).count();
                let run_end = pos + run;
                if run >= 2 {
                    let prev = src[..pos].chars().next_back();
                    let next = src[run_end..].chars().next();
                    let can_open = !next.map_or(true, char::is_whitespace);
                    let can_close = !prev.map_or(true, char::is_whitespace);
                    let marker = if b == b'~' {
                        DelimMarker::Strike
                    } else {
                        DelimMarker::Spoiler
                    };
                    let mut rem = run;
                    while rem >= 2 {
                        tokens.push(Token::Delim {
                            marker,
                            can_open,
                            can_close,
                            matched: false,
                        });
                        rem -= 2;
                    }
                    if rem == 1 {
                        push_literal_char(&mut tokens, b as char);
                    }
                } else {
                    push_literal_char(&mut tokens, b as char);
                }
                pos = run_end;
            }
            _ => {
                let ch = src[pos..].chars().next().unwrap();
                push_literal_char(&mut tokens, ch);
                pos += ch.len_utf8();
            }
        }
    }

    let mut pairs: Vec<(usize, usize, EntityKind)> = Vec::new();
    for close_idx in 0..tokens.len() {
        let (close_marker, can_close, close_matched) = match &tokens[close_idx] {
            Token::Delim { marker, can_close, matched, .. } => (*marker, *can_close, *matched),
            Token::Literal { .. } => continue,
        };
        if !can_close || close_matched {
            continue;
        }
        let mut found = None;
        for open_idx in (0..close_idx).rev() {
            if let Token::Delim { marker, can_open, matched, .. } = &tokens[open_idx] {
                let (open_marker, can_open, open_matched) = (*marker, *can_open, *matched);
                if !open_matched && can_open && open_marker == close_marker {
                    if has_nonempty_literal(&tokens[open_idx + 1..close_idx]) {
                        found = Some(open_idx);
                    }
                    break;
                }
            }
        }
        if let Some(open_idx) = found {
            if let Token::Delim { matched, .. } = &mut tokens[open_idx] {
                *matched = true;
            }
            if let Token::Delim { matched, .. } = &mut tokens[close_idx] {
                *matched = true;
            }
            pairs.push((open_idx, close_idx, close_marker.kind()));
        }
    }

    let mut out = String::with_capacity(src.len());
    let mut entities: Vec<RichEntity> = Vec::new();
    let mut starts = vec![0usize; tokens.len()];
    let mut ends = vec![0usize; tokens.len()];
    for (idx, tok) in tokens.iter().enumerate() {
        let start = utf16_len(&out);
        starts[idx] = start;
        match tok {
            Token::Literal { text, extra_entities } => {
                for e in extra_entities {
                    entities.push(RichEntity {
                        offset: start + e.offset,
                        length: e.length,
                        kind: e.kind.clone(),
                    });
                }
                out.push_str(text);
            }
            Token::Delim { marker, matched, .. } => {
                if !matched {
                    out.push_str(marker.raw());
                }
            }
        }
        ends[idx] = utf16_len(&out);
    }

    for (open_idx, close_idx, kind) in pairs {
        let start = ends[open_idx];
        let end = starts[close_idx];
        if end > start {
            entities.push(RichEntity {
                offset: start,
                length: end - start,
                kind,
            });
        }
    }

    normalize_entities(&out, &mut entities);
    RichText { text: out, entities }
}

fn append_inline(slice: &str, out: &mut String, entities: &mut Vec<RichEntity>) {
    if slice.is_empty() {
        return;
    }
    let base = utf16_len(out);
    let parsed = parse_inline(slice);
    for mut e in parsed.entities {
        e.offset += base;
        entities.push(e);
    }
    out.push_str(&parsed.text);
}

pub(super) fn from_markdown(input: &str) -> RichText {
    let bytes = input.as_bytes();
    let mut pos = 0usize;
    let mut inline_start = 0usize;
    let mut out = String::with_capacity(input.len());
    let mut entities = Vec::new();

    while pos < bytes.len() {
        if bytes[pos] == b'\\' {
            if let Some(next) = input[pos + 1..].chars().next() {
                if is_escapable(next) {
                    pos += 1 + next.len_utf8();
                    continue;
                }
            }
            pos += 1;
            continue;
        }
        if bytes[pos] == b'`' {
            let run = bytes[pos..].iter().take_while(|&&b| b == b'`').count();
            if run == 3 {
                if let Some(rel) = input[pos + 3..].find("```") {
                    let inner = &input[pos + 3..pos + 3 + rel];
                    if let Some((lang, body)) = split_fenced_code(inner) {
                        append_inline(&input[inline_start..pos], &mut out, &mut entities);
                        let offset = utf16_len(&out);
                        out.push_str(body);
                        let length = utf16_len(body);
                        entities.push(RichEntity {
                            offset,
                            length,
                            kind: EntityKind::Pre(lang),
                        });
                        pos = pos + 3 + rel + 3;
                        inline_start = pos;
                        continue;
                    }
                    pos = pos + 3 + rel + 3;
                    continue;
                }
                pos += run;
                continue;
            }
            if run == 1 {
                if let Some(rel) = find_inline_code_end(&input[pos + 1..]) {
                    pos = pos + 1 + rel + 1;
                    continue;
                }
            }
            pos += run;
            continue;
        }
        let at_line_start = pos == 0 || bytes[pos - 1] == b'\n';
        if at_line_start && input[pos..].starts_with("> ") {
            let mut cur = pos;
            let mut lines: Vec<&str> = Vec::new();
            let mut last_had_newline = false;
            while cur < input.len() && input[cur..].starts_with("> ") {
                let content_start = cur + 2;
                let (line_end, next_cur, had_nl) = match input[content_start..].find('\n') {
                    Some(rel) => {
                        let end = content_start + rel;
                        (end, end + 1, true)
                    }
                    None => (input.len(), input.len(), false),
                };
                let line = input[content_start..line_end]
                    .strip_suffix('\r')
                    .unwrap_or(&input[content_start..line_end]);
                lines.push(line);
                last_had_newline = had_nl;
                cur = next_cur;
            }
            let joined = lines.join("\n");
            if !joined.trim().is_empty() {
                append_inline(&input[inline_start..pos], &mut out, &mut entities);
                let bq_offset = utf16_len(&out);
                let inner = parse_inline(&joined);
                for mut e in inner.entities {
                    e.offset += bq_offset;
                    entities.push(e);
                }
                out.push_str(&inner.text);
                let bq_len = utf16_len(&inner.text);
                if bq_len > 0 {
                    entities.push(RichEntity {
                        offset: bq_offset,
                        length: bq_len,
                        kind: EntityKind::BlockQuote,
                    });
                }
                if last_had_newline {
                    out.push('\n');
                }
                pos = cur;
                inline_start = pos;
                continue;
            }
            pos = cur;
            continue;
        }
        let ch = input[pos..].chars().next().unwrap();
        pos += ch.len_utf8();
    }

    append_inline(&input[inline_start..], &mut out, &mut entities);
    normalize_entities(&out, &mut entities);
    RichText { text: out, entities }
}

fn bare_url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i:(?:https?|tg)://\S+)").expect("bare_url_re"))
}

fn num_unit_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            r"[0-9]+(?:[.,][0-9]+)*(?:",
            "[ \u{00A0}]?", r"(?:°[CFcf]|℃|℉|°|%|‰|₫|\$|€|£|¥)",
            "|", "[ \u{00A0}]?", r"(?:(?i:độ)[ \u{00A0}]+[CFcf]|(?i:độ|giờ|phút|giây|ngày|tuần|tháng|năm|tiếng|lít|mét|tấn|tạ|yến|lạng|gam|đồng))",
            "|", "[ \u{00A0}]?", r"(?i:km/h|m/s|[µμ]g/m³|ug/m3|km²|km2|cm²|cm2|mm²|mm2|m²|m2|m³|m3|cm³|cm3)",
            "|", "[ \u{00A0}]?", r"(?i:kWh|MWh|Wh|GHz|MHz|kHz|Hz|Gbps|Mbps|kbps|bps|GiB|MiB|KiB|TB|GB|MB|KB|hPa|dBm|dB|ppm|ppb|lux|lx|lm|rpm|fps|psi|bar|VND|VNĐ|USD)",
            "|", "[ \u{00A0}]?", r"(?:kVA|VA|Pa|MW|mW|kV|mV|mA)",
            "|", "[ \u{00A0}]?", r"(?i:km|cm|mm|[µμ]m|nm|kg|mg|ml|cl|dl|ha|ms|kW|hrs|hr|mins|min|secs|sec|px|đ)",
            "|", "[ \u{00A0}]?", r"(?:[VAWL]|[mglhs])",
            ")"
        ))
        .expect("num_unit_re")
    })
}

struct NormMap {
    norm: String,
    cluster_starts: Vec<Option<usize>>,
    cluster_ends: Vec<Option<usize>>,
}

impl NormMap {
    fn new(text: &str) -> Self {
        let mut norm = String::with_capacity(text.len());
        let mut cluster_starts = Vec::with_capacity(text.len() + 1);
        let mut cluster_ends = Vec::with_capacity(text.len() + 1);

        let mut chars = text.char_indices().peekable();
        let mut orig_u16 = 0usize;

        while let Some((start_byte, first_ch)) = chars.next() {
            let mut end_byte = start_byte + first_ch.len_utf8();
            let mut cluster_u16 = first_ch.len_utf16();
            while let Some(&(next_byte, next_ch)) = chars.peek() {
                if !is_combining_mark(next_ch) {
                    break;
                }
                chars.next();
                end_byte = next_byte + next_ch.len_utf8();
                cluster_u16 += next_ch.len_utf16();
            }
            let cluster = &text[start_byte..end_byte];
            let norm_start = norm.len();
            for c in cluster.nfc() {
                norm.push(c);
            }
            let norm_end = norm.len();
            if cluster_starts.len() <= norm_end {
                cluster_starts.resize(norm_end + 1, None);
                cluster_ends.resize(norm_end + 1, None);
            }
            if cluster_starts[norm_start].is_none() {
                cluster_starts[norm_start] = Some(orig_u16);
            }
            cluster_ends[norm_end] = Some(orig_u16 + cluster_u16);
            orig_u16 += cluster_u16;
        }
        if cluster_starts.is_empty() {
            cluster_starts.push(Some(0));
            cluster_ends.push(Some(0));
        }
        Self { norm, cluster_starts, cluster_ends }
    }
}

pub(super) fn apply_auto_emphasis(text: &str, entities: &mut Vec<RichEntity>) {
    if text.is_empty() || entities.len() >= MAX_ENTITIES {
        return;
    }
    let map = NormMap::new(text);
    let norm = &map.norm;
    let url_spans: Vec<(usize, usize)> = bare_url_re()
        .find_iter(norm)
        .map(|m| (m.start(), m.end()))
        .collect();
    let existing: Vec<(usize, usize)> = entities
        .iter()
        .map(|e| (e.offset, e.offset + e.length))
        .collect();

    for m in num_unit_re().find_iter(norm) {
        let mut start = m.start();
        let end = m.end();

        let prev = norm[..start].chars().next_back();
        if matches!(prev, Some('+' | '-')) {
            let before_sign = start - 1;
            let prev2 = norm[..before_sign].chars().next_back();
            if prev2.is_none()
                || prev2.is_some_and(|c| c.is_whitespace() || matches!(c, '(' | '[' | '{' | ':' | ',' | ';' | '='))
            {
                start = before_sign;
            } else if prev == Some('-') && prev2.is_some_and(char::is_alphabetic) {
                continue;
            }
        } else if prev.is_some_and(|c| is_word_char(c) || matches!(c, '_' | '.')) {
            continue;
        }

        let next = norm[end..].chars().next();
        if next.is_some_and(|c| is_word_char(c) || c == '_') {
            continue;
        }

        if url_spans.iter().any(|&(us, ue)| start < ue && us < end) {
            continue;
        }

        let (Some(u16_start), Some(u16_end)) = (
            map.cluster_starts.get(start).copied().flatten(),
            map.cluster_ends.get(end).copied().flatten(),
        ) else {
            continue;
        };
        if u16_end <= u16_start {
            continue;
        }
        if existing.iter().any(|&(es, ee)| u16_start < ee && es < u16_end) {
            continue;
        }
        entities.push(RichEntity {
            offset: u16_start,
            length: u16_end - u16_start,
            kind: EntityKind::Bold,
        });
        if entities.len() >= MAX_ENTITIES {
            break;
        }
    }
}

fn is_emoji_base(c: char) -> bool {
    if is_skin_tone(c) {
        return false;
    }
    let u = c as u32;
    matches!(
        u,
        0x1F300..=0x1FAFF
            | 0x1F1E6..=0x1F1FF
            | 0x2600..=0x27BF
            | 0x2B05..=0x2B55
            | 0x231A..=0x231B
            | 0x23E9..=0x23FA
    )
}

fn is_regional_indicator(c: char) -> bool {
    matches!(c as u32, 0x1F1E6..=0x1F1FF)
}

fn is_skin_tone(c: char) -> bool {
    matches!(c as u32, 0x1F3FB..=0x1F3FF)
}

fn is_variation_selector(c: char) -> bool {
    matches!(c as u32, 0xFE0E | 0xFE0F)
}

fn consume_emoji_cluster(chars: &[char], idx: usize) -> Option<usize> {
    let &first = chars.get(idx)?;
    if !is_emoji_base(first) {
        return None;
    }
    let mut i = idx + 1;
    if is_regional_indicator(first) {
        if chars.get(i).copied().is_some_and(is_regional_indicator) {
            return Some(i + 1);
        }
        return None;
    }
    const MAX_ZWJ_JOINS: usize = 4;
    let mut zwj_joins = 0usize;
    loop {
        if chars.get(i).copied().is_some_and(is_variation_selector) {
            i += 1;
        }
        if chars.get(i).copied().is_some_and(is_skin_tone) {
            i += 1;
        }
        if chars.get(i).copied().is_some_and(is_variation_selector) {
            i += 1;
        }
        if zwj_joins < MAX_ZWJ_JOINS && chars.get(i) == Some(&'\u{200D}') {
            if let Some(&next) = chars.get(i + 1) {
                if is_emoji_base(next) && !is_regional_indicator(next) {
                    i += 2;
                    zwj_joins += 1;
                    continue;
                }
            }
        }
        break;
    }
    Some(i)
}

fn consume_emoji_insertion(orig_next: Option<char>, mark_chars: &[char], mi: usize) -> Option<usize> {
    if mark_chars.get(mi) == Some(&' ') {
        return consume_emoji_cluster(mark_chars, mi + 1);
    }
    let mut after = consume_emoji_cluster(mark_chars, mi)?;
    if mark_chars.get(after) == Some(&' ') && orig_next != Some(' ') {
        after += 1;
    }
    Some(after)
}

pub(super) fn preserves_original_text(original: &str, marked_plain: &str) -> bool {
    let orig_chars: Vec<char> = original.nfc().collect();
    let mark_chars: Vec<char> = marked_plain.nfc().collect();
    if orig_chars == mark_chars {
        return true;
    }
    let mut oi = 0usize;
    let mut mi = 0usize;
    let mut inserted_emojis = 0usize;
    while oi < orig_chars.len() || mi < mark_chars.len() {
        if oi < orig_chars.len() && mi < mark_chars.len() && orig_chars[oi] == mark_chars[mi] {
            if let Some(m_end) = consume_emoji_cluster(&mark_chars, mi) {
                if consume_emoji_cluster(&orig_chars, oi)
                    .is_some_and(|o_end| orig_chars[oi..o_end] == mark_chars[mi..m_end])
                {
                    oi += m_end - mi;
                    mi = m_end;
                    continue;
                }
            } else {
                oi += 1;
                mi += 1;
                continue;
            }
        }
        let orig_next = orig_chars.get(oi).copied();
        let Some(next_mi) = consume_emoji_insertion(orig_next, &mark_chars, mi) else {
            return false;
        };
        inserted_emojis += 1;
        if inserted_emojis > MAX_AI_INSERTED_EMOJIS {
            return false;
        }
        mi = next_mi;
    }
    true
}

fn normalize_entities(text: &str, entities: &mut Vec<RichEntity>) {
    let units: Vec<u16> = text.encode_utf16().collect();
    let total = units.len();
    entities.retain(|e| {
        e.length > 0
            && e.offset.checked_add(e.length).is_some_and(|end| {
                end <= total && is_utf16_boundary(&units, e.offset) && is_utf16_boundary(&units, end)
            })
    });
    entities.sort_by(|a, b| {
        a.offset
            .cmp(&b.offset)
            .then_with(|| Reverse(a.length).cmp(&Reverse(b.length)))
            .then_with(|| a.kind.rank().cmp(&b.kind.rank()))
            .then_with(|| a.kind.extra_key().cmp(b.kind.extra_key()))
    });
    entities.dedup();
    entities.truncate(MAX_ENTITIES);
}

pub(super) fn render(text: &str, opts: &RichOptions<'_>) -> Result<RichText, GwError> {
    for raw in std::iter::once(text).chain(opts.ai_markup) {
        if raw.len() > MAX_RAW_UTF16 * 3 || utf16_len(raw) > MAX_RAW_UTF16 {
            return Err(GwError::bad_request("Tin quá dài (tối đa 4096 đơn vị UTF-16 chữ, kể cả dấu định dạng không quá 16384)."));
        }
    }
    let mut rich = if let Some(markup) = opts.ai_markup {
        let parsed = from_markdown(markup);
        if !preserves_original_text(text, &parsed.text) {
            return Err(GwError::new(
                422,
                "AI_MARKUP_MISMATCH",
                "ai_markup làm thay đổi nội dung chữ gốc.",
            ));
        }
        parsed
    } else {
        match opts.format {
            FormatMode::Markdown => from_markdown(text),
            FormatMode::Plain => RichText {
                text: text.to_owned(),
                entities: Vec::new(),
            },
        }
    };

    if utf16_len(&rich.text) > MAX_TEXT_UTF16 {
        return Err(GwError::bad_request("Tin quá dài (tối đa 4096 đơn vị UTF-16)."));
    }

    if opts.auto_emphasis {
        normalize_entities(&rich.text, &mut rich.entities);
        apply_auto_emphasis(&rich.text, &mut rich.entities);
    }
    normalize_entities(&rich.text, &mut rich.entities);
    Ok(rich)
}

pub(super) fn to_tdlib_formatted_text(rich: &RichText) -> Value {
    let entities: Vec<Value> = rich
        .entities
        .iter()
        .map(|e| {
            json!({
                "@type": "textEntity",
                "offset": e.offset,
                "length": e.length,
                "type": e.kind.to_tdlib_type(),
            })
        })
        .collect();
    json!({
        "@type": "formattedText",
        "text": rich.text,
        "entities": entities,
    })
}

pub(super) fn render_tdlib(text: &str, opts: &RichOptions<'_>) -> Result<Value, GwError> {
    let rich = render(text, opts)?;
    Ok(to_tdlib_formatted_text(&rich))
}
