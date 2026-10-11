pub const MAX_STRING: usize = 4096;
pub const MAX_OUTPUT: usize = 8 * 1024 * 1024;
pub const MAX_CELLS: usize = 2_000_000;
#[derive(Debug, PartialEq)]
pub enum Value {
    Missing,
    Number(f64),
    Text(String),
    ExcelDate {
        serial: f64,
        epoch_1904: bool,
        duration: bool,
    },
}
#[derive(Debug, PartialEq)]
pub struct Cell {
    pub row: u32,
    pub column: u32,
    pub flags: u8,
    pub value: Value,
}
pub struct Decoder {
    range: [u32; 4],
    buffer: Vec<u8>,
    cells: Vec<Cell>,
    total: usize,
    started: bool,
    ended: bool,
    last: Option<(u32, u32)>,
    owned: usize,
    source_identity: Option<[u8; 32]>,
    worksheet_index: Option<u32>,
    epoch_1904: bool,
}
impl Value {
    pub fn iso_text(&self) -> Result<String, &'static str> {
        let Self::ExcelDate {
            serial,
            epoch_1904,
            duration,
        } = self
        else {
            return Err("expected Excel date or duration");
        };
        if !serial.is_finite() {
            return Err("nonfinite date or duration");
        }
        if *duration {
            let seconds = *serial * 86400.0;
            if !seconds.is_finite() {
                return Err("duration overflow");
            }
            return Ok(format!(
                "{}PT{:.3}S",
                if seconds.is_sign_negative() { "-" } else { "" },
                seconds.abs()
            ));
        }
        if *serial < 0.0
            || *serial
                >= if *epoch_1904 {
                    2_957_004.0
                } else {
                    2_958_466.0
                }
        {
            return Err("date outside supported Excel range");
        }
        if !epoch_1904 && serial.floor() == 60.0 {
            return Err("Excel 1900-02-29 is fictitious; explicit missing/text choice required");
        }
        let date = calamine::ExcelDateTime::new(
            *serial,
            calamine::ExcelDateTimeType::DateTime,
            *epoch_1904,
        );
        let (year, month, day, hour, minute, second, millis) = date.to_ymd_hms_milli();
        if (year, month, day) == (1900, 2, 29) {
            return Err("date rounded onto Excel's fictitious leap day");
        }
        Ok(format!(
            "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}"
        ))
    }
}
pub fn validate_range(range: [u32; 4]) -> Result<(), &'static str> {
    let [r0, c0, r1, c1] = range;
    if r0 > r1
        || c0 > c1
        || r1 >= 1_048_576
        || c1 >= 16384
        || (u64::from(r1 - r0) + 1) * (u64::from(c1 - c0) + 1) > MAX_CELLS as u64
    {
        Err("invalid or oversized selected rectangle")
    } else {
        Ok(())
    }
}
impl Decoder {
    pub fn new(range: [u32; 4]) -> Result<Self, &'static str> {
        validate_range(range)?;
        Ok(Self {
            range,
            buffer: Vec::with_capacity(MAX_STRING + 15 + 4096),
            cells: Vec::new(),
            total: 0,
            started: false,
            ended: false,
            last: None,
            owned: 0,
            source_identity: None,
            worksheet_index: None,
            epoch_1904: false,
        })
    }
    pub fn for_source(
        range: [u32; 4],
        source_identity: [u8; 32],
        worksheet_index: u32,
    ) -> Result<Self, &'static str> {
        let mut decoder = Self::new(range)?;
        decoder.source_identity = Some(source_identity);
        decoder.worksheet_index = Some(worksheet_index);
        Ok(decoder)
    }
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
        self.total = self
            .total
            .checked_add(bytes.len())
            .ok_or("output size overflow")?;
        if self.total > MAX_OUTPUT || self.buffer.len() + bytes.len() > MAX_STRING + 15 + 4096 {
            return Err("output exceeds budget");
        }
        if self.ended && !bytes.is_empty() {
            return Err("trailing output");
        }
        self.buffer.extend_from_slice(bytes);
        if !self.started {
            if self.buffer.len() < 41 {
                return Ok(());
            }
            if &self.buffer[..4] != b"XLP2" {
                return Err("invalid protocol version");
            }
            let identity: [u8; 32] = self.buffer[4..36].try_into().unwrap();
            let index = u32::from_le_bytes(self.buffer[36..40].try_into().unwrap());
            if self
                .source_identity
                .is_some_and(|expected| identity != expected)
                || self
                    .worksheet_index
                    .is_some_and(|expected| index != expected)
                || self.buffer[40] > 1
            {
                return Err("worker source or worksheet identity mismatch");
            }
            self.epoch_1904 = self.buffer[40] == 1;
            self.buffer.drain(..41);
            self.started = true;
        }
        loop {
            if self.buffer.first() == Some(&255) {
                if self.buffer.len() != 1 {
                    return Err("trailing output");
                }
                self.buffer.clear();
                self.ended = true;
                return Ok(());
            }
            if self.buffer.is_empty() {
                return Ok(());
            }
            if self.buffer[0] != 0 {
                return Err("invalid record marker");
            }
            if self.buffer.len() < 15 {
                return Ok(());
            }
            let row = u32::from_le_bytes(self.buffer[1..5].try_into().unwrap());
            let column = u32::from_le_bytes(self.buffer[5..9].try_into().unwrap());
            let flags = self.buffer[9];
            let tag = self.buffer[10];
            let len = u32::from_le_bytes(self.buffer[11..15].try_into().unwrap()) as usize;
            if len > MAX_STRING {
                return Err("cell payload too large");
            }
            if self.buffer.len() < 15 + len {
                return Ok(());
            }
            let [r0, c0, r1, c1] = self.range;
            if row < r0
                || row > r1
                || column < c0
                || column > c1
                || self.last.is_some_and(|last| last >= (row, column))
            {
                return Err("invalid cell coordinate or ordering");
            }
            if flags & !59 != 0
                || flags & 2 != 0 && (flags & 1 == 0 || tag != 0)
                || flags & 8 != 0 && tag != 3 && tag != 4
                || flags & 16 != 0 && (tag != 3 || flags & 8 != 0)
                || flags & 32 != 0 && tag != 1
                || tag == 1 && flags & 32 == 0
                || tag == 0 && (flags & 1 != 0) != (flags & 2 != 0)
                || (tag == 3 || tag == 4) && (flags & 8 != 0) != self.epoch_1904
            {
                return Err("invalid cell flags");
            }
            let width = std::mem::size_of::<Cell>();
            let next_capacity = if self.cells.len() == self.cells.capacity() {
                self.cells.capacity().max(1) * 2
            } else {
                self.cells.capacity()
            };
            let retained = next_capacity
                .checked_mul(width)
                .and_then(|n| n.checked_add(self.owned))
                .and_then(|n| n.checked_add(if tag == 2 { len } else { 0 }));
            if self.cells.len() >= MAX_CELLS || retained.is_none_or(|n| n > MAX_OUTPUT) {
                return Err("retained output exceeds budget");
            }
            let payload = &self.buffer[15..15 + len];
            let value = match tag {
                0 if len == 0 => Value::Missing,
                1 | 3 | 4 if len == 8 => {
                    let number = f64::from_le_bytes(payload.try_into().unwrap());
                    if !number.is_finite() {
                        return Err("nonfinite number");
                    }
                    if (flags & 16 != 0) != (tag == 3 && flags & 8 == 0 && number.floor() == 60.0) {
                        return Err("invalid fictitious date flag");
                    }
                    if tag == 3
                        && (number < 0.0
                            || number
                                >= if flags & 8 != 0 {
                                    2_957_004.0
                                } else {
                                    2_958_466.0
                                })
                    {
                        return Err("date outside supported Excel range");
                    }
                    if tag == 1 {
                        Value::Number(number)
                    } else {
                        Value::ExcelDate {
                            serial: number,
                            epoch_1904: flags & 8 != 0,
                            duration: tag == 4,
                        }
                    }
                }
                2 => Value::Text(
                    std::str::from_utf8(payload)
                        .map_err(|_| "invalid UTF-8")?
                        .to_owned(),
                ),
                _ => return Err("invalid cell type or length"),
            };
            if tag == 2 {
                self.owned += len;
            }
            if self.cells.len() == self.cells.capacity() {
                self.cells.reserve_exact(next_capacity - self.cells.len());
            }
            self.cells.push(Cell {
                row,
                column,
                flags,
                value,
            });
            self.last = Some((row, column));
            self.buffer.drain(..15 + len);
        }
    }
    pub fn finish(self) -> Result<Vec<Cell>, &'static str> {
        if !self.started || !self.ended || !self.buffer.is_empty() {
            Err("incomplete worker output")
        } else {
            Ok(self.cells)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(row: u32, column: u32, flags: u8, tag: u8, value: &[u8]) -> Vec<u8> {
        let mut out = Vec::from(&b"XLP2"[..]);
        out.extend([0; 37]);
        out.push(0);
        out.extend(row.to_le_bytes());
        out.extend(column.to_le_bytes());
        out.extend([flags, tag]);
        out.extend((value.len() as u32).to_le_bytes());
        out.extend(value);
        out.push(255);
        out
    }
    #[test]
    fn valid_fragmented_text_and_formula_cache() {
        let mut decoder = Decoder::new([0, 0, 2, 2]).unwrap();
        for byte in packet(1, 1, 1, 2, b"00123") {
            decoder.feed(&[byte]).unwrap();
        }
        let cells = decoder.finish().unwrap();
        assert_eq!(cells[0].value, Value::Text("00123".into()));
        assert_eq!(cells[0].flags, 1);
    }
    #[test]
    fn rejects_dimensions_payload_flags_and_unfinished_output() {
        assert!(Decoder::new([0, 0, 1_048_575, 16383]).is_err());
        for bytes in [
            packet(3, 0, 0, 2, b"x"),
            packet(0, 0, 128, 2, b"x"),
            packet(0, 0, 0, 2, &vec![b'x'; 4097]),
            packet(0, 0, 0, 1, &f64::NAN.to_le_bytes()),
            packet(0, 0, 2, 2, b"x"),
        ] {
            let mut d = Decoder::new([0, 0, 2, 2]).unwrap();
            assert!(d.feed(&bytes).is_err());
        }
        let mut d = Decoder::new([0, 0, 2, 2]).unwrap();
        d.feed(b"XLP2").unwrap();
        assert!(d.finish().is_err());
    }
    #[test]
    fn rejects_duplicate_cells_and_trailing_data() {
        let bytes = packet(0, 0, 0, 2, b"x");
        let mut duplicate = bytes[..bytes.len() - 1].to_vec();
        duplicate.extend(&bytes[41..]);
        let mut d = Decoder::new([0, 0, 2, 2]).unwrap();
        assert!(d.feed(&duplicate).is_err());
        let mut d = Decoder::new([0, 0, 2, 2]).unwrap();
        d.feed(&bytes).unwrap();
        assert!(d.feed(b"extra").is_err());
    }
    #[test]
    fn row_255_does_not_collide_with_end_marker() {
        let mut d = Decoder::new([255, 0, 255, 0]).unwrap();
        d.feed(&packet(255, 0, 0, 2, b"x")).unwrap();
        assert_eq!(d.finish().unwrap()[0].row, 255);
    }
    #[test]
    fn rejects_missing_required_precision_cache_and_fictitious_date_flags() {
        for bytes in [
            packet(0, 0, 0, 1, &1.0f64.to_le_bytes()),
            packet(0, 0, 1, 0, b""),
            packet(0, 0, 0, 3, &60.0f64.to_le_bytes()),
        ] {
            let mut decoder = Decoder::new([0, 0, 0, 0]).unwrap();
            assert!(decoder.feed(&bytes).is_err());
        }
    }
    #[test]
    fn date_iso_conversion_refuses_fake_day_and_preserves_epochs() {
        let value = |serial, epoch_1904, duration| Value::ExcelDate {
            serial,
            epoch_1904,
            duration,
        };
        assert_eq!(
            value(61.5, false, false).iso_text().unwrap(),
            "1900-03-01T12:00:00.000"
        );
        assert_eq!(
            value(0.0, true, false).iso_text().unwrap(),
            "1904-01-01T00:00:00.000"
        );
        assert!(value(60.0, false, false).iso_text().is_err());
        assert_eq!(value(1.5, false, true).iso_text().unwrap(), "PT129600.000S");
    }
    #[test]
    fn contextual_decoder_rejects_changed_source_or_selected_worksheet() {
        let mut d = Decoder::for_source([0, 0, 0, 0], [42; 32], 0).unwrap();
        assert!(d.feed(&packet(0, 0, 0, 2, b"x")).is_err());
        let mut d = Decoder::for_source([0, 0, 0, 0], [0; 32], 1).unwrap();
        assert!(d.feed(&packet(0, 0, 0, 2, b"x")).is_err());
    }
}
