use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
use tessera_workbook::supervisor::{run, Config};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let worker = args
        .next()
        .ok_or("expected worker path, workbook path and optional timeout ms")?
        .into();
    let source = args.next().ok_or("workbook path missing")?.into();
    let timeout = args.next().map(|n| n.parse()).transpose()?.unwrap_or(3000);
    let cells = run(Config {
        worker,
        source,
        range: [0, 0, 99, 99],
        worksheet_index: 0,
        quota: 2 * 1024 * 1024,
        retained_bytes: 0,
        timeout: Duration::from_millis(timeout),
        cancel: Arc::new(AtomicBool::new(false)),
        probe: None,
    })?;
    println!(
        "Accepted {} cells after successful worker completion.",
        cells.len()
    );
    Ok(())
}
