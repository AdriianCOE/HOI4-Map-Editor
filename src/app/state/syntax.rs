use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewlineStyle {
    None,
    Lf,
    Crlf,
    Cr,
    Mixed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceText {
    pub path: PathBuf,
    pub text: String,
    pub bom: bool,
    pub newline_style: NewlineStyle,
    pub original_len: usize,
}

impl SourceText {
    pub fn new(path: impl Into<PathBuf>, text: impl Into<String>) -> Self {
        let text = text.into();
        let original_len = text.len();
        let bom = text.starts_with('\u{feff}');
        let newline_style = detect_newline_style(&text);

        Self {
            path: path.into(),
            text,
            bom,
            newline_style,
            original_len,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextSpan {
    pub start: usize,
    pub len: usize,
}

impl TextSpan {
    pub fn new(start: usize, len: usize) -> Self {
        Self { start, len }
    }

    pub fn end(self) -> usize {
        self.start.saturating_add(self.len)
    }

    pub fn len(self) -> usize {
        self.len
    }

    pub fn is_empty(self) -> bool {
        self.len == 0
    }

    pub fn line_column(self, source: &SourceText) -> LineColumn {
        line_column_for_offset(&source.text, self.start)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineColumn {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Identifier,
    String,
    Integer,
    Decimal,
    Equals,
    LeftBrace,
    RightBrace,
    Comment,
    Whitespace,
    Newline,
    Unknown,
    EndOfFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: TextSpan,
    pub text: String,
    pub line: usize,
    pub column: usize,
    pub terminated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyntaxDiagnosticKind {
    UnexpectedRightBrace,
    UnclosedBlock,
    UnclosedString,
    EqualsWithoutValue,
    UnknownToken,
    EmptyFile,
    NestingLimitExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxDiagnostic {
    pub kind: SyntaxDiagnosticKind,
    pub span: TextSpan,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ParseOptions {
    pub nesting_limit: usize,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self { nesting_limit: 128 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PdxDocument {
    pub source: SourceText,
    pub tokens: Vec<Token>,
    pub entries: Vec<PdxEntry>,
    pub diagnostics: Vec<SyntaxDiagnostic>,
    pub span: TextSpan,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PdxBlock {
    pub entries: Vec<PdxEntry>,
    pub span: TextSpan,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PdxEntry {
    pub key: Option<PdxScalar>,
    pub value: PdxValue,
    pub span: TextSpan,
    pub equals_span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PdxValue {
    Block(PdxBlock),
    Scalar(PdxScalar),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PdxScalar {
    pub kind: PdxScalarKind,
    pub text: String,
    pub span: TextSpan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdxScalarKind {
    Identifier,
    String,
    Integer,
    Decimal,
    Unknown,
}

pub fn lex_text(path: impl Into<PathBuf>, text: impl Into<String>) -> Vec<Token> {
    let source = SourceText::new(path, text);
    lex(&source)
}

pub fn lex(source: &SourceText) -> Vec<Token> {
    Lexer::new(source).lex()
}

pub fn parse_text(path: impl Into<PathBuf>, text: impl Into<String>) -> PdxDocument {
    parse(SourceText::new(path, text))
}

pub fn parse(source: SourceText) -> PdxDocument {
    parse_with_options(source, ParseOptions::default())
}

pub fn parse_with_options(source: SourceText, options: ParseOptions) -> PdxDocument {
    let tokens = lex(&source);
    Parser::new(source, tokens, options).parse()
}

struct Lexer<'a> {
    source: &'a SourceText,
    cursor: usize,
    tokens: Vec<Token>,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a SourceText) -> Self {
        Self {
            source,
            cursor: 0,
            tokens: Vec::new(),
        }
    }

    fn lex(mut self) -> Vec<Token> {
        while self.cursor < self.source.text.len() {
            let before = self.cursor;
            self.lex_one();
            if self.cursor <= before {
                self.cursor = next_offset(&self.source.text, before);
            }
        }

        let location = line_column_for_offset(&self.source.text, self.cursor);
        self.tokens.push(Token {
            kind: TokenKind::EndOfFile,
            span: TextSpan::new(self.cursor, 0),
            text: String::new(),
            line: location.line,
            column: location.column,
            terminated: true,
        });

        self.tokens
    }

    fn lex_one(&mut self) {
        let Some((_, ch)) = self
            .source
            .text
            .get(self.cursor..)
            .and_then(|rest| rest.char_indices().next())
        else {
            return;
        };

        match ch {
            '=' => self.push_fixed(TokenKind::Equals, ch.len_utf8(), true),
            '{' => self.push_fixed(TokenKind::LeftBrace, ch.len_utf8(), true),
            '}' => self.push_fixed(TokenKind::RightBrace, ch.len_utf8(), true),
            '"' => self.lex_string(),
            '#' => self.lex_comment(),
            '\r' | '\n' => self.lex_newline(),
            ch if ch.is_whitespace() || ch == '\u{feff}' => self.lex_whitespace(),
            ch if is_identifier_start(ch) || ch == '-' || ch.is_ascii_digit() => self.lex_atom(),
            _ => self.push_fixed(TokenKind::Unknown, ch.len_utf8(), true),
        }
    }

    fn push_fixed(&mut self, kind: TokenKind, len: usize, terminated: bool) {
        let start = self.cursor;
        self.cursor = self.cursor.saturating_add(len);
        self.push_token(kind, start, self.cursor, terminated);
    }

    fn push_token(&mut self, kind: TokenKind, start: usize, end: usize, terminated: bool) {
        let location = line_column_for_offset(&self.source.text, start);
        let text = self
            .source
            .text
            .get(start..end)
            .map_or_else(String::new, str::to_string);
        self.tokens.push(Token {
            kind,
            span: TextSpan::new(start, end.saturating_sub(start)),
            text,
            line: location.line,
            column: location.column,
            terminated,
        });
    }

    fn lex_string(&mut self) {
        let start = self.cursor;
        self.cursor = next_offset(&self.source.text, self.cursor);
        let mut escaped = false;
        let mut terminated = false;

        while let Some((offset, ch)) = char_at_or_after(&self.source.text, self.cursor) {
            self.cursor = offset.saturating_add(ch.len_utf8());
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                terminated = true;
                break;
            }
            if ch == '\r' || ch == '\n' {
                self.cursor = offset;
                break;
            }
        }

        self.push_token(TokenKind::String, start, self.cursor, terminated);
    }

    fn lex_comment(&mut self) {
        let start = self.cursor;
        while let Some((offset, ch)) = char_at_or_after(&self.source.text, self.cursor) {
            if ch == '\r' || ch == '\n' {
                self.cursor = offset;
                break;
            }
            self.cursor = offset.saturating_add(ch.len_utf8());
        }
        self.push_token(TokenKind::Comment, start, self.cursor, true);
    }

    fn lex_newline(&mut self) {
        let start = self.cursor;
        let Some((_, ch)) = char_at_or_after(&self.source.text, self.cursor) else {
            return;
        };

        self.cursor = self.cursor.saturating_add(ch.len_utf8());
        if ch == '\r' {
            if let Some((_, next)) = char_at_or_after(&self.source.text, self.cursor) {
                if next == '\n' {
                    self.cursor = self.cursor.saturating_add(next.len_utf8());
                }
            }
        }
        self.push_token(TokenKind::Newline, start, self.cursor, true);
    }

    fn lex_whitespace(&mut self) {
        let start = self.cursor;
        while let Some((offset, ch)) = char_at_or_after(&self.source.text, self.cursor) {
            if ch == '\r' || ch == '\n' || (!ch.is_whitespace() && ch != '\u{feff}') {
                self.cursor = offset;
                break;
            }
            self.cursor = offset.saturating_add(ch.len_utf8());
        }
        self.push_token(TokenKind::Whitespace, start, self.cursor, true);
    }

    fn lex_atom(&mut self) {
        let start = self.cursor;
        while let Some((offset, ch)) = char_at_or_after(&self.source.text, self.cursor) {
            if ch.is_whitespace() || matches!(ch, '=' | '{' | '}' | '"' | '#') {
                self.cursor = offset;
                break;
            }
            self.cursor = offset.saturating_add(ch.len_utf8());
        }

        let text = self
            .source
            .text
            .get(start..self.cursor)
            .map_or("", |text| text);
        self.push_token(classify_atom(text), start, self.cursor, true);
    }
}

struct Parser {
    source: SourceText,
    tokens: Vec<Token>,
    cursor: usize,
    diagnostics: Vec<SyntaxDiagnostic>,
    options: ParseOptions,
}

impl Parser {
    fn new(source: SourceText, tokens: Vec<Token>, options: ParseOptions) -> Self {
        Self {
            source,
            tokens,
            cursor: 0,
            diagnostics: Vec::new(),
            options,
        }
    }

    fn parse(mut self) -> PdxDocument {
        let span = TextSpan::new(0, self.source.text.len());
        let entries = self.parse_entries(0, false);

        if entries.is_empty() && !self.has_non_trivia() {
            self.diagnostic(
                SyntaxDiagnosticKind::EmptyFile,
                TextSpan::new(0, 0),
                "empty file",
            );
        }

        PdxDocument {
            source: self.source,
            tokens: self.tokens,
            entries,
            diagnostics: self.diagnostics,
            span,
        }
    }

    fn parse_entries(&mut self, depth: usize, stop_on_right_brace: bool) -> Vec<PdxEntry> {
        let mut entries = Vec::new();

        loop {
            self.skip_trivia();
            let token = self.current().cloned();
            let Some(token) = token else {
                break;
            };

            match token.kind {
                TokenKind::EndOfFile => break,
                TokenKind::RightBrace => {
                    if stop_on_right_brace {
                        break;
                    }
                    self.diagnostic(
                        SyntaxDiagnosticKind::UnexpectedRightBrace,
                        token.span,
                        "unexpected right brace",
                    );
                    self.advance();
                }
                _ => {
                    if let Some(entry) = self.parse_entry(depth) {
                        entries.push(entry);
                    } else {
                        self.advance();
                    }
                }
            }
        }

        entries
    }

    fn parse_entry(&mut self, depth: usize) -> Option<PdxEntry> {
        let first = self.parse_value(depth)?;
        self.skip_trivia();

        let Some(equals) = self
            .current()
            .cloned()
            .filter(|token| token.kind == TokenKind::Equals)
        else {
            let span = value_span(&first);
            return Some(PdxEntry {
                key: None,
                value: first,
                span,
                equals_span: None,
            });
        };

        self.advance();
        self.skip_trivia();

        let key = scalar_from_value(first);
        let value = match self.current().cloned() {
            Some(token) if is_value_start(token.kind) => self.parse_value(depth),
            _ => None,
        };

        let Some(value) = value else {
            self.diagnostic(
                SyntaxDiagnosticKind::EqualsWithoutValue,
                equals.span,
                "equals without value",
            );
            let fallback = PdxValue::Scalar(PdxScalar {
                kind: PdxScalarKind::Unknown,
                text: String::new(),
                span: TextSpan::new(equals.span.end(), 0),
            });
            let start = key_span_start(&key, equals.span.start);
            return Some(PdxEntry {
                key,
                value: fallback,
                span: TextSpan::new(start, equals.span.end().saturating_sub(start)),
                equals_span: Some(equals.span),
            });
        };

        let start = key_span_start(&key, equals.span.start);
        let end = value_span(&value).end();
        Some(PdxEntry {
            key,
            value,
            span: TextSpan::new(start, end.saturating_sub(start)),
            equals_span: Some(equals.span),
        })
    }

    fn parse_value(&mut self, depth: usize) -> Option<PdxValue> {
        self.skip_trivia();
        let token = self.current().cloned()?;

        match token.kind {
            TokenKind::LeftBrace => Some(PdxValue::Block(self.parse_block(depth))),
            TokenKind::Identifier
            | TokenKind::String
            | TokenKind::Integer
            | TokenKind::Decimal
            | TokenKind::Unknown => {
                self.advance();
                if token.kind == TokenKind::String && !token.terminated {
                    self.diagnostic(
                        SyntaxDiagnosticKind::UnclosedString,
                        token.span,
                        "unclosed string",
                    );
                }
                if token.kind == TokenKind::Unknown {
                    self.diagnostic(
                        SyntaxDiagnosticKind::UnknownToken,
                        token.span,
                        "unknown token",
                    );
                }
                Some(PdxValue::Scalar(PdxScalar {
                    kind: scalar_kind_for_token(token.kind),
                    text: token.text,
                    span: token.span,
                }))
            }
            TokenKind::RightBrace => {
                self.diagnostic(
                    SyntaxDiagnosticKind::UnexpectedRightBrace,
                    token.span,
                    "unexpected right brace",
                );
                self.advance();
                None
            }
            TokenKind::Equals => {
                self.diagnostic(
                    SyntaxDiagnosticKind::EqualsWithoutValue,
                    token.span,
                    "equals without value",
                );
                self.advance();
                None
            }
            _ => None,
        }
    }

    fn parse_block(&mut self, depth: usize) -> PdxBlock {
        let open = self.current().cloned();
        if open.is_some() {
            self.advance();
        }

        let open_span = open.as_ref().map(|token| token.span).unwrap_or_default();
        if depth >= self.options.nesting_limit {
            self.diagnostic(
                SyntaxDiagnosticKind::NestingLimitExceeded,
                open_span,
                "nesting limit exceeded",
            );
        }

        let entries = if depth >= self.options.nesting_limit {
            self.skip_until_matching_right_brace();
            Vec::new()
        } else {
            self.parse_entries(depth.saturating_add(1), true)
        };

        self.skip_trivia();
        let close = self
            .current()
            .cloned()
            .filter(|token| token.kind == TokenKind::RightBrace);
        if close.is_some() {
            self.advance();
        } else {
            self.diagnostic(
                SyntaxDiagnosticKind::UnclosedBlock,
                open_span,
                "unclosed block",
            );
        }

        let end = match close.as_ref() {
            Some(token) => token.span.end(),
            None => entries
                .last()
                .map(|entry| entry.span.end())
                .unwrap_or_else(|| open_span.end()),
        };

        PdxBlock {
            entries,
            span: TextSpan::new(open_span.start, end.saturating_sub(open_span.start)),
        }
    }

    fn skip_until_matching_right_brace(&mut self) {
        let mut depth = 0usize;
        while let Some(token) = self.current().cloned() {
            match token.kind {
                TokenKind::EndOfFile => break,
                TokenKind::LeftBrace => {
                    depth = depth.saturating_add(1);
                    self.advance();
                }
                TokenKind::RightBrace => {
                    if depth == 0 {
                        break;
                    }
                    depth = depth.saturating_sub(1);
                    self.advance();
                }
                _ => self.advance(),
            }
        }
    }

    fn skip_trivia(&mut self) {
        while self
            .current()
            .map(|token| is_trivia(token.kind))
            .is_some_and(|is_trivia| is_trivia)
        {
            self.advance();
        }
    }

    fn has_non_trivia(&self) -> bool {
        self.tokens
            .iter()
            .any(|token| !is_trivia(token.kind) && token.kind != TokenKind::EndOfFile)
    }

    fn current(&self) -> Option<&Token> {
        self.tokens.get(self.cursor)
    }

    fn advance(&mut self) {
        if self.cursor < self.tokens.len() {
            self.cursor = self.cursor.saturating_add(1);
        }
    }

    fn diagnostic(&mut self, kind: SyntaxDiagnosticKind, span: TextSpan, message: &str) {
        self.diagnostics.push(SyntaxDiagnostic {
            kind,
            span,
            message: message.to_string(),
        });
    }
}

fn detect_newline_style(text: &str) -> NewlineStyle {
    let mut has_lf = false;
    let mut has_crlf = false;
    let mut has_cr = false;
    let mut iter = text.chars().peekable();

    while let Some(ch) = iter.next() {
        if ch == '\r' {
            if iter.peek().copied() == Some('\n') {
                iter.next();
                has_crlf = true;
            } else {
                has_cr = true;
            }
        } else if ch == '\n' {
            has_lf = true;
        }
    }

    match (has_lf, has_crlf, has_cr) {
        (false, false, false) => NewlineStyle::None,
        (true, false, false) => NewlineStyle::Lf,
        (false, true, false) => NewlineStyle::Crlf,
        (false, false, true) => NewlineStyle::Cr,
        _ => NewlineStyle::Mixed,
    }
}

fn line_column_for_offset(text: &str, offset: usize) -> LineColumn {
    let mut line = 1usize;
    let mut column = 1usize;
    let target = offset.min(text.len());
    let mut iter = text.char_indices().peekable();

    while let Some((idx, ch)) = iter.next() {
        if idx >= target {
            break;
        }
        let mut cursor = idx.saturating_add(ch.len_utf8());
        if ch == '\r' {
            if iter.peek().map(|(_, next)| *next) == Some('\n') {
                if let Some((next_idx, next)) = iter.next() {
                    cursor = next_idx.saturating_add(next.len_utf8());
                }
            }
            line = line.saturating_add(1);
            column = 1;
        } else if ch == '\n' {
            line = line.saturating_add(1);
            column = 1;
        } else {
            column = column.saturating_add(1);
        }
        if cursor >= target {
            break;
        }
    }

    LineColumn { line, column }
}

fn char_at_or_after(text: &str, offset: usize) -> Option<(usize, char)> {
    text.get(offset..)
        .and_then(|rest| rest.char_indices().next())
        .map(|(relative, ch)| (offset.saturating_add(relative), ch))
}

fn next_offset(text: &str, offset: usize) -> usize {
    match char_at_or_after(text, offset) {
        Some((_, ch)) => offset.saturating_add(ch.len_utf8()),
        None => text.len(),
    }
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_alphabetic()
}

fn classify_atom(text: &str) -> TokenKind {
    if is_integer(text) {
        TokenKind::Integer
    } else if is_decimal(text) {
        TokenKind::Decimal
    } else {
        TokenKind::Identifier
    }
}

fn is_integer(text: &str) -> bool {
    let digits = text.strip_prefix('-').map_or(text, |digits| digits);
    !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit())
}

fn is_decimal(text: &str) -> bool {
    let digits = text.strip_prefix('-').map_or(text, |digits| digits);
    let mut dot_seen = false;
    let mut digit_seen = false;

    for ch in digits.chars() {
        if ch == '.' && !dot_seen {
            dot_seen = true;
        } else if ch.is_ascii_digit() {
            digit_seen = true;
        } else {
            return false;
        }
    }

    dot_seen && digit_seen
}

fn is_trivia(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Whitespace | TokenKind::Newline | TokenKind::Comment
    )
}

fn is_value_start(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Identifier
            | TokenKind::String
            | TokenKind::Integer
            | TokenKind::Decimal
            | TokenKind::Unknown
            | TokenKind::LeftBrace
    )
}

fn scalar_kind_for_token(kind: TokenKind) -> PdxScalarKind {
    match kind {
        TokenKind::String => PdxScalarKind::String,
        TokenKind::Integer => PdxScalarKind::Integer,
        TokenKind::Decimal => PdxScalarKind::Decimal,
        TokenKind::Unknown => PdxScalarKind::Unknown,
        _ => PdxScalarKind::Identifier,
    }
}

fn value_span(value: &PdxValue) -> TextSpan {
    match value {
        PdxValue::Block(block) => block.span,
        PdxValue::Scalar(scalar) => scalar.span,
    }
}

fn scalar_from_value(value: PdxValue) -> Option<PdxScalar> {
    match value {
        PdxValue::Scalar(scalar) => Some(scalar),
        PdxValue::Block(_) => None,
    }
}

fn key_span_start(key: &Option<PdxScalar>, fallback: usize) -> usize {
    key.as_ref()
        .map(|scalar| scalar.span.start)
        .map_or(fallback, |start| start)
}

#[cfg(test)]
mod tests {
    use super::{
        NewlineStyle, ParseOptions, PdxValue, SourceText, SyntaxDiagnosticKind, TokenKind,
        lex_text, parse_text, parse_with_options,
    };

    #[test]
    fn lexer_preserves_trivia_numbers_and_eof() {
        let tokens = lex_text("state.txt", "id = 1 # ok\r\nfactor=-2.5\nname=\"A\"");
        let kinds: Vec<TokenKind> = tokens.iter().map(|token| token.kind).collect();

        assert_eq!(
            kinds,
            vec![
                TokenKind::Identifier,
                TokenKind::Whitespace,
                TokenKind::Equals,
                TokenKind::Whitespace,
                TokenKind::Integer,
                TokenKind::Whitespace,
                TokenKind::Comment,
                TokenKind::Newline,
                TokenKind::Identifier,
                TokenKind::Equals,
                TokenKind::Decimal,
                TokenKind::Newline,
                TokenKind::Identifier,
                TokenKind::Equals,
                TokenKind::String,
                TokenKind::EndOfFile
            ]
        );
        assert_eq!(
            tokens.get(10).map(|token| token.text.as_str()),
            Some("-2.5")
        );
    }

    #[test]
    fn source_text_tracks_bom_newlines_and_safe_line_column() {
        let source = SourceText::new("state.txt", "\u{feff}a\r\nb\n");
        let span = super::TextSpan::new(6, 2);
        let location = span.line_column(&source);

        assert!(source.bom);
        assert_eq!(source.newline_style, NewlineStyle::Mixed);
        assert_eq!(location.line, 2);
        assert_eq!(location.column, 1);
    }

    #[test]
    fn parser_preserves_duplicate_keys_and_positional_values() {
        let document = parse_text("state.txt", "resources={ steel=5 steel=6 42 }");
        let root = document.entries.first();
        let block = root.and_then(|entry| match &entry.value {
            PdxValue::Block(block) => Some(block),
            _ => None,
        });

        assert!(document.diagnostics.is_empty());
        assert_eq!(document.entries.len(), 1);
        assert_eq!(block.map(|block| block.entries.len()), Some(3));
        assert_eq!(
            block
                .and_then(|block| block.entries.get(1))
                .and_then(|entry| entry.key.as_ref())
                .map(|key| key.text.as_str()),
            Some("steel")
        );
        assert!(
            block
                .and_then(|block| block.entries.get(2))
                .and_then(|entry| entry.key.as_ref())
                .is_none()
        );
    }

    #[test]
    fn parser_recovers_common_errors() {
        let document = parse_text("bad.txt", "} a= b= \"open\n c={ d=1");
        let kinds: Vec<SyntaxDiagnosticKind> = document
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.kind.clone())
            .collect();

        assert!(kinds.contains(&SyntaxDiagnosticKind::UnexpectedRightBrace));
        assert!(kinds.contains(&SyntaxDiagnosticKind::EqualsWithoutValue));
        assert!(kinds.contains(&SyntaxDiagnosticKind::UnclosedString));
        assert!(kinds.contains(&SyntaxDiagnosticKind::UnclosedBlock));
        assert!(!document.entries.is_empty());
    }

    #[test]
    fn parser_reports_empty_unknown_and_nesting_limit() {
        let empty = parse_text("empty.txt", " \n # comment");
        let unknown = parse_text("unknown.txt", "@");
        let nested = parse_with_options(
            SourceText::new("nested.txt", "a={ b={ c=1 } }"),
            ParseOptions { nesting_limit: 1 },
        );

        assert_eq!(
            empty
                .diagnostics
                .first()
                .map(|diagnostic| diagnostic.kind.clone()),
            Some(SyntaxDiagnosticKind::EmptyFile)
        );
        assert_eq!(
            unknown
                .diagnostics
                .first()
                .map(|diagnostic| diagnostic.kind.clone()),
            Some(SyntaxDiagnosticKind::UnknownToken)
        );
        assert!(
            nested
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == SyntaxDiagnosticKind::NestingLimitExceeded)
        );
    }
}
