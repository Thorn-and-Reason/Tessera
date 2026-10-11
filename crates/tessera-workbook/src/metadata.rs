pub const MAX_METADATA: usize = 64 * 1024;
pub const MAX_SHEETS: usize = 256;
pub const MAX_SHEET_NAME: usize = 1024;
#[derive(Debug, PartialEq)]
pub struct Worksheet {
    pub index: u32,
    pub name: String,
    /// 0 worksheet, 1 chart sheet, 2 dialog sheet, 3 macro sheet, 4 VBA module.
    pub kind: u8,
    /// 0 visible, 1 hidden, 2 very hidden.
    pub visibility: u8,
    pub dimensions: Option<[u32; 4]>,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(name: &[u8]) -> Vec<u8> {
        let mut bytes = b"XLM1".to_vec();
        bytes.extend([0; 33]);
        bytes.push(0);
        bytes.extend(0u32.to_le_bytes());
        bytes.extend([0, 0, 1]);
        bytes.extend([0; 16]);
        bytes.extend((name.len() as u16).to_le_bytes());
        bytes.extend(name);
        bytes.push(255);
        bytes
    }
    #[test]
    fn fragmented_metadata_and_wrong_identity_or_name_are_validated() {
        let mut d = MetadataDecoder::new([0; 32]);
        for byte in packet(b"Data") {
            d.feed(&[byte]).unwrap();
        }
        let info = d.finish().unwrap();
        assert_eq!(info.sheets[0].dimensions, Some([0; 4]));
        assert!(info.owned_bytes() < MAX_METADATA);
        for name in [
            b"\xff".as_slice(),
            b"bad\nname".as_slice(),
            &vec![b'x'; 1025],
        ] {
            let mut d = MetadataDecoder::new([0; 32]);
            assert!(d.feed(&packet(name)).is_err());
        }
        let mut d = MetadataDecoder::new([1; 32]);
        assert!(d.feed(&packet(b"Data")).is_err());
    }
}
#[derive(Debug, PartialEq)]
pub struct WorkbookInfo {
    pub source_identity: [u8; 32],
    pub epoch_1904: bool,
    pub sheets: Vec<Worksheet>,
}
impl WorkbookInfo {
    pub fn owned_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.sheets.capacity() * std::mem::size_of::<Worksheet>()
            + self.sheets.iter().map(|s| s.name.capacity()).sum::<usize>()
    }
}
pub struct MetadataDecoder {
    expected: [u8; 32],
    buffer: Vec<u8>,
    info: WorkbookInfo,
    total: usize,
    started: bool,
    ended: bool,
    names_bytes: usize,
}
impl MetadataDecoder {
    pub fn new(expected: [u8; 32]) -> Self {
        Self {
            expected,
            buffer: Vec::with_capacity(MAX_SHEET_NAME + 26 + 4096),
            info: WorkbookInfo {
                source_identity: expected,
                epoch_1904: false,
                sheets: Vec::new(),
            },
            total: 0,
            started: false,
            ended: false,
            names_bytes: 0,
        }
    }
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
        self.total = self
            .total
            .checked_add(bytes.len())
            .ok_or("metadata overflow")?;
        if self.total > MAX_METADATA || self.buffer.len() + bytes.len() > MAX_SHEET_NAME + 26 + 4096
        {
            return Err("metadata output exceeds budget");
        }
        if self.ended && !bytes.is_empty() {
            return Err("trailing metadata");
        }
        self.buffer.extend_from_slice(bytes);
        if !self.started {
            if self.buffer.len() < 37 {
                return Ok(());
            }
            if &self.buffer[..4] != b"XLM1"
                || self.buffer[4..36] != self.expected
                || self.buffer[36] > 1
            {
                return Err("metadata identity or version mismatch");
            }
            self.info.epoch_1904 = self.buffer[36] == 1;
            self.buffer.drain(..37);
            self.started = true;
        }
        loop {
            if self.buffer.is_empty() {
                return Ok(());
            }
            if self.buffer[0] == 255 {
                if self.buffer.len() != 1 {
                    return Err("trailing metadata");
                }
                self.buffer.clear();
                self.ended = true;
                return Ok(());
            }
            if self.buffer[0] != 0 {
                return Err("invalid metadata record");
            }
            if self.buffer.len() < 26 {
                return Ok(());
            }
            let index = u32::from_le_bytes(self.buffer[1..5].try_into().unwrap());
            let kind = self.buffer[5];
            let visibility = self.buffer[6];
            let present = self.buffer[7];
            let len = u16::from_le_bytes(self.buffer[24..26].try_into().unwrap()) as usize;
            if len == 0
                || len > MAX_SHEET_NAME
                || kind > 4
                || visibility > 2
                || present > 1
                || index as usize != self.info.sheets.len()
                || self.info.sheets.len() >= MAX_SHEETS
            {
                return Err("invalid or oversized worksheet metadata");
            }
            if self.buffer.len() < 26 + len {
                return Ok(());
            }
            let mut dims = [0; 4];
            for (i, value) in dims.iter_mut().enumerate() {
                *value = u32::from_le_bytes(self.buffer[8 + 4 * i..12 + 4 * i].try_into().unwrap());
            }
            if present == 1
                && (kind != 0
                    || dims[0] > dims[2]
                    || dims[1] > dims[3]
                    || dims[2] >= 1_048_576
                    || dims[3] >= 16384)
                || present == 0 && dims != [0; 4]
            {
                return Err("invalid worksheet dimensions");
            }
            let name = std::str::from_utf8(&self.buffer[26..26 + len])
                .map_err(|_| "invalid worksheet name UTF-8")?;
            if name.chars().any(char::is_control) || self.info.sheets.iter().any(|s| s.name == name)
            {
                return Err("invalid or duplicate worksheet name");
            }
            let capacity = if self.info.sheets.len() == self.info.sheets.capacity() {
                self.info.sheets.capacity().max(1) * 2
            } else {
                self.info.sheets.capacity()
            };
            if capacity * std::mem::size_of::<Worksheet>()
                + self.names_bytes
                + len
                + std::mem::size_of::<WorkbookInfo>()
                > MAX_METADATA
            {
                return Err("retained metadata exceeds budget");
            }
            let name = name.to_owned();
            self.names_bytes += len;
            if capacity > self.info.sheets.capacity() {
                self.info
                    .sheets
                    .reserve_exact(capacity - self.info.sheets.len());
            }
            self.info.sheets.push(Worksheet {
                index,
                name,
                kind,
                visibility,
                dimensions: if present == 1 { Some(dims) } else { None },
            });
            self.buffer.drain(..26 + len);
        }
    }
    pub fn finish(self) -> Result<WorkbookInfo, &'static str> {
        if !self.started || !self.ended || !self.buffer.is_empty() || self.info.sheets.is_empty() {
            Err("incomplete workbook metadata")
        } else {
            Ok(self.info)
        }
    }
}
