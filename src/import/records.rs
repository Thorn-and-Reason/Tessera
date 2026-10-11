use super::{check_bytes, Delimiter};
use crate::{
    error::{AppError, AppResult},
    operation::JobControl,
};
use std::borrow::Cow;

pub(crate) struct Field<'a> {
    pub text: Cow<'a, str>,
    pub row: usize,
    pub column: usize,
    pub end_record: bool,
}
impl Field<'_> {
    pub fn invalid(&self, message: &str) -> AppError {
        invalid(self.row, self.column, message)
    }
}
fn invalid(row: usize, column: usize, message: &str) -> AppError {
    AppError::invalid(format!("row {row}, field {column}"), message)
}

pub(crate) struct Scanner<'a> {
    bytes: &'a [u8],
    delimiter: u8,
    position: usize,
    row: usize,
    column: usize,
    finished: bool,
    control: &'a JobControl,
    next_checkpoint: usize,
    pub budget_bytes: usize,
}
impl<'a> Scanner<'a> {
    pub fn new(
        bytes: &'a [u8],
        delimiter: Delimiter,
        control: &'a JobControl,
        budget_bytes: usize,
    ) -> AppResult<Self> {
        control.checkpoint(0, Some(bytes.len()))?;
        let position = if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
            3
        } else {
            0
        };
        Ok(Self {
            bytes,
            delimiter: match delimiter {
                Delimiter::Comma => b',',
                Delimiter::Tab => b'\t',
            },
            position,
            row: 1,
            column: 1,
            finished: position == bytes.len(),
            control,
            next_checkpoint: 4096,
            budget_bytes,
        })
    }
    fn advance(&mut self, count: usize) -> AppResult<()> {
        self.position += count;
        if self.position >= self.next_checkpoint {
            self.control
                .checkpoint(self.position, Some(self.bytes.len()))?;
            self.next_checkpoint = self.position + 4096;
        }
        Ok(())
    }
    fn decode(&self, start: usize, end: usize, escaped: usize) -> AppResult<Cow<'a, str>> {
        let text = std::str::from_utf8(&self.bytes[start..end])
            .map_err(|_| invalid(self.row, self.column, "Input must be valid UTF-8."))?;
        if escaped == 0 {
            return Ok(Cow::Borrowed(text));
        }
        check_bytes(self.budget_bytes, text.len() - escaped)?;
        let mut decoded = String::with_capacity(text.len() - escaped);
        let mut pieces = text.split("\"\"");
        decoded.push_str(pieces.next().unwrap_or(""));
        for piece in pieces {
            decoded.push('"');
            decoded.push_str(piece);
        }
        Ok(Cow::Owned(decoded))
    }
    fn quoted(&mut self) -> AppResult<Cow<'a, str>> {
        self.advance(1)?;
        let start = self.position;
        let mut escaped = 0;
        while self.position < self.bytes.len() {
            if self.bytes[self.position] == b'"' {
                if self.bytes.get(self.position + 1) == Some(&b'"') {
                    escaped += 1;
                    self.advance(2)?;
                    continue;
                }
                let end = self.position;
                self.advance(1)?;
                return self.decode(start, end, escaped);
            }
            self.advance(1)?;
        }
        Err(invalid(
            self.row,
            self.column,
            "Quoted field is not closed.",
        ))
    }
    fn unquoted(&mut self) -> AppResult<Cow<'a, str>> {
        let start = self.position;
        while let Some(&byte) = self.bytes.get(self.position) {
            if byte == self.delimiter || matches!(byte, b'\r' | b'\n') {
                break;
            }
            if byte == b'"' {
                return Err(invalid(
                    self.row,
                    self.column,
                    "Quotes must enclose the whole field.",
                ));
            }
            self.advance(1)?;
        }
        self.decode(start, self.position, 0)
    }
    pub fn next(&mut self) -> AppResult<Option<Field<'a>>> {
        if self.finished {
            return Ok(None);
        }
        let text = if self.bytes.get(self.position) == Some(&b'"') {
            self.quoted()?
        } else {
            self.unquoted()?
        };
        let row = self.row;
        let column = self.column;
        let end_record = self.finish_field()?;
        Ok(Some(Field {
            text,
            row,
            column,
            end_record,
        }))
    }
    fn finish_field(&mut self) -> AppResult<bool> {
        match self.bytes.get(self.position).copied() {
            Some(byte) if byte == self.delimiter => {
                self.advance(1)?;
                self.column += 1;
                Ok(false)
            }
            Some(b'\r' | b'\n') => {
                let crlf = self.bytes[self.position] == b'\r'
                    && self.bytes.get(self.position + 1) == Some(&b'\n');
                self.advance(if crlf { 2 } else { 1 })?;
                self.row += 1;
                self.column = 1;
                self.finished = self.position == self.bytes.len();
                Ok(true)
            }
            None => {
                self.finished = true;
                Ok(true)
            }
            _ => Err(invalid(
                self.row,
                self.column,
                "Only a delimiter or line ending may follow a closing quote.",
            )),
        }
    }
}
