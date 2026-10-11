#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use calamine::{DataRef, Reader, SheetType, SheetVisible, Xlsx};
use sha2::{Digest, Sha256};
use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use tessera_workbook::metadata::{MAX_METADATA, MAX_SHEETS, MAX_SHEET_NAME};
use tessera_workbook::protocol::{validate_range, MAX_OUTPUT, MAX_STRING};

fn probe(mode: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut out = std::io::stdout().lock();
    match mode {
        "sleep" => std::thread::sleep(std::time::Duration::from_secs(30)),
        "flood" => loop {
            out.write_all(&[0; 4096])?;
        },
        "partial" => out.write_all(b"XLP1")?,
        "bad-coordinate" => {
            out.write_all(b"XLP1")?;
            out.write_all(&[0])?;
            out.write_all(&u32::MAX.to_le_bytes())?;
            out.write_all(&[0; 10])?;
        }
        _ => return Err("unknown probe".into()),
    }
    Ok(())
}

fn emit_protocol(
    mut workbook: Xlsx<std::io::Cursor<Vec<u8>>>,
    range: [u32; 4],
    source_identity: [u8; 32],
    worksheet_index: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    validate_range(range)?;
    let epoch = workbook.has_1904_epoch();
    let sheet = workbook
        .sheets_metadata()
        .get(worksheet_index as usize)
        .ok_or("worksheet index out of bounds")?;
    if sheet.typ != SheetType::WorkSheet {
        return Err("selected sheet is not a worksheet".into());
    }
    let name = sheet.name.clone();
    let mut reader = workbook.worksheet_cells_reader(&name)?;
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    out.write_all(b"XLP2")?;
    out.write_all(&source_identity)?;
    out.write_all(&worksheet_index.to_le_bytes())?;
    out.write_all(&[u8::from(epoch)])?;
    let mut total = 42usize;
    while let Some(record) = reader.next_cell_with_formula_metadata()? {
        let (row, column) = record.pos;
        let [r0, c0, r1, c1] = range;
        if row < r0 || row > r1 || column < c0 || column > c1 {
            continue;
        }
        let mut flags = u8::from(record.formula.is_some());
        let (tag, payload) = match record.value {
            DataRef::Empty => {
                if flags == 1 {
                    flags |= 2;
                }
                (0, Vec::new())
            }
            DataRef::Float(n) => {
                if !n.is_finite() {
                    return Err("nonfinite value".into());
                }
                flags |= 32;
                (1, n.to_le_bytes().to_vec())
            }
            DataRef::Int(n) => (2, n.to_string().into_bytes()),
            DataRef::String(s) | DataRef::DateTimeIso(s) | DataRef::DurationIso(s) => {
                (2, s.into_bytes())
            }
            DataRef::SharedString(s) => (2, s.as_bytes().to_vec()),
            DataRef::Bool(v) => (2, v.to_string().into_bytes()),
            DataRef::DateTime(date) => {
                let serial = date.as_f64();
                if !serial.is_finite() {
                    return Err("nonfinite date".into());
                }
                if epoch {
                    flags |= 8;
                }
                let tag = if date.is_duration() { 4 } else { 3 };
                if tag == 3 && !epoch && serial.floor() == 60.0 {
                    flags |= 16;
                }
                (tag, serial.to_le_bytes().to_vec())
            }
            DataRef::Error(_) => {
                return Err("Excel error cell requires an explicit import policy".into())
            }
        };
        if payload.len() > MAX_STRING {
            return Err("cell payload exceeds protocol ceiling".into());
        }
        total = total
            .checked_add(15 + payload.len())
            .ok_or("output overflow")?;
        if total > MAX_OUTPUT {
            return Err("output exceeds protocol ceiling".into());
        }
        out.write_all(&[0])?;
        out.write_all(&row.to_le_bytes())?;
        out.write_all(&column.to_le_bytes())?;
        out.write_all(&[flags, tag])?;
        out.write_all(&(payload.len() as u32).to_le_bytes())?;
        out.write_all(&payload)?;
    }
    out.write_all(&[255])?;
    out.flush()?;
    Ok(())
}
fn emit_metadata(
    mut workbook: Xlsx<std::io::Cursor<Vec<u8>>>,
    identity: [u8; 32],
) -> Result<(), Box<dyn std::error::Error>> {
    let count = workbook.sheets_metadata().len();
    if count == 0 || count > MAX_SHEETS {
        return Err("invalid or oversized sheet count".into());
    }
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    out.write_all(b"XLM1")?;
    out.write_all(&identity)?;
    out.write_all(&[u8::from(workbook.has_1904_epoch())])?;
    let mut total = 38usize;
    for index in 0..count {
        let sheet = &workbook.sheets_metadata()[index];
        if sheet.name.is_empty() || sheet.name.len() > MAX_SHEET_NAME {
            return Err("oversized or missing sheet name".into());
        }
        let name = sheet.name.clone();
        let kind = match sheet.typ {
            SheetType::WorkSheet => 0,
            SheetType::ChartSheet => 1,
            SheetType::DialogSheet => 2,
            SheetType::MacroSheet => 3,
            SheetType::Vba => 4,
        };
        let visibility = match sheet.visible {
            SheetVisible::Visible => 0,
            SheetVisible::Hidden => 1,
            SheetVisible::VeryHidden => 2,
        };
        let dims = if kind == 0 {
            let reader = workbook.worksheet_cells_reader(&name)?;
            let d = reader.dimensions();
            Some([d.start.0, d.start.1, d.end.0, d.end.1])
        } else {
            None
        };
        total += 26 + name.len();
        if total > MAX_METADATA {
            return Err("metadata wire budget exceeded".into());
        }
        out.write_all(&[0])?;
        out.write_all(&(index as u32).to_le_bytes())?;
        out.write_all(&[kind, visibility, u8::from(dims.is_some())])?;
        for coordinate in dims.unwrap_or([0; 4]) {
            out.write_all(&coordinate.to_le_bytes())?;
        }
        out.write_all(&(name.len() as u16).to_le_bytes())?;
        out.write_all(name.as_bytes())?;
    }
    out.write_all(&[255])?;
    out.flush()?;
    Ok(())
}

struct QuotaAllocator;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static LIMIT: AtomicUsize = AtomicUsize::new(16 * 1024 * 1024);

fn charge(bytes: usize) -> bool {
    let result = LIVE.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |live| {
        live.checked_add(bytes)
            .filter(|n| *n <= LIMIT.load(Ordering::SeqCst))
    });
    if let Ok(old) = result {
        PEAK.fetch_max(old + bytes, Ordering::SeqCst);
        true
    } else {
        false
    }
}

unsafe impl GlobalAlloc for QuotaAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !charge(layout.size()) {
            return std::ptr::null_mut();
        }
        let ptr = System.alloc(layout);
        if ptr.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !charge(layout.size()) {
            return std::ptr::null_mut();
        }
        let ptr = System.alloc_zeroed(layout);
        if ptr.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Charge a full new block: realloc may briefly own old and new storage together.
        if !charge(new_size) {
            return std::ptr::null_mut();
        }
        let next = System.realloc(ptr, layout, new_size);
        if next.is_null() {
            LIVE.fetch_sub(new_size, Ordering::SeqCst);
        } else {
            LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
        }
        next
    }
}

#[global_allocator]
static ALLOCATOR: QuotaAllocator = QuotaAllocator;

fn allocator_self_check() {
    let baseline = LIVE.load(Ordering::SeqCst);
    LIMIT.store(baseline + 256, Ordering::SeqCst);
    unsafe {
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = ALLOCATOR.alloc_zeroed(layout);
        assert!(!ptr.is_null());
        assert_eq!(LIVE.load(Ordering::SeqCst), baseline + 64);
        assert_eq!(*ptr, 0);
        let ptr = ALLOCATOR.realloc(ptr, layout, 128);
        assert!(!ptr.is_null());
        assert_eq!(LIVE.load(Ordering::SeqCst), baseline + 128);
        let layout = Layout::from_size_align(128, 8).unwrap();
        assert!(ALLOCATOR.realloc(ptr, layout, 256).is_null());
        assert_eq!(LIVE.load(Ordering::SeqCst), baseline + 128);
        ALLOCATOR.dealloc(ptr, layout);
        assert_eq!(LIVE.load(Ordering::SeqCst), baseline);
        let impossible = Layout::from_size_align(257, 8).unwrap();
        assert!(ALLOCATOR.alloc(impossible).is_null());
        assert_eq!(LIVE.load(Ordering::SeqCst), baseline);
    }
    LIMIT.store(16 * 1024 * 1024, Ordering::SeqCst);
    println!("{{\"allocator_self_check\":true}}");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let first = args
        .next()
        .ok_or("expected fixture path and optional heap quota")?;
    if first == "--probe" {
        return probe(&args.next().ok_or("probe mode missing")?);
    }
    let protocol = first == "--protocol";
    let inspect = first == "--inspect";
    let path = if protocol || inspect {
        args.next().ok_or("protocol source missing")?
    } else {
        first
    };
    if path == "--allocator-self-check" {
        allocator_self_check();
        return Ok(());
    }
    let quota = args
        .next()
        .map(|n| n.parse())
        .transpose()?
        .unwrap_or(16 * 1024 * 1024);
    let worksheet_index = if protocol {
        args.next().ok_or("worksheet index missing")?.parse()?
    } else {
        0
    };
    let range = if protocol {
        let mut range = [0u32; 4];
        for coordinate in &mut range {
            *coordinate = args.next().ok_or("selected rectangle missing")?.parse()?;
        }
        Some(range)
    } else {
        None
    };
    if quota < LIVE.load(Ordering::SeqCst) {
        return Err("quota below startup allocations".into());
    }
    LIMIT.store(quota, Ordering::SeqCst);
    let source = std::fs::read(path)?;
    let source_bytes = source.len();
    let source_identity = Sha256::digest(&source).into();
    let mut workbook = Xlsx::new(std::io::Cursor::new(source))?;
    if inspect {
        return emit_metadata(workbook, source_identity);
    }
    if let Some(range) = range {
        return emit_protocol(workbook, range, source_identity, worksheet_index);
    }
    let name = workbook.sheet_names().first().ok_or("no sheet")?.clone();
    let mut reader = workbook.worksheet_cells_reader(&name)?;
    let mut cells = 0usize;
    let mut formulas = 0usize;
    let mut value_bytes = 0usize;
    let mut max_row = 0u32;
    let mut max_column = 0u32;
    while let Some(record) = reader.next_cell_with_formula_metadata()? {
        cells = cells.checked_add(1).ok_or("cell count overflow")?;
        formulas += usize::from(record.formula.is_some());
        max_row = max_row.max(record.pos.0);
        max_column = max_column.max(record.pos.1);
        let bytes = match record.value {
            DataRef::String(s) | DataRef::DateTimeIso(s) | DataRef::DurationIso(s) => s.len(),
            DataRef::SharedString(s) => s.len(),
            DataRef::Float(_) | DataRef::Int(_) | DataRef::DateTime(_) => 8,
            DataRef::Bool(_) => 1,
            DataRef::Error(_) | DataRef::Empty => 0,
        };
        value_bytes = value_bytes
            .checked_add(bytes)
            .ok_or("value byte overflow")?;
    }
    println!("{{\"cells\":{cells},\"formulas\":{formulas},\"max_row_zero_based\":{max_row},\"max_column_zero_based\":{max_column},\"source_bytes\":{source_bytes},\"decoded_value_bytes\":{value_bytes},\"heap_live\":{},\"heap_peak\":{},\"heap_quota\":{quota}}}", LIVE.load(Ordering::SeqCst), PEAK.load(Ordering::SeqCst));
    Ok(())
}
