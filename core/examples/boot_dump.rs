//! Boots the ROM headless and writes the screen as a PBM image.
//! Usage: cargo run --profile test --example boot_dump -- <seconds> <out.pbm> [rom.bin]
//! The ROM defaults to resources/roms/ekta37.bin.
fn main() {
    let mut args = std::env::args().skip(1);
    let secs: f64 = args.next().map_or(3.0, |s| s.parse().unwrap());
    let out = args.next().unwrap_or_else(|| "screen.pbm".into());
    let rom_path = args
        .next()
        .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../resources/roms/ekta37.bin").into());
    let rom = std::fs::read(&rom_path).unwrap();
    let mut m = juku::Machine::new(&rom).unwrap();
    m.run_cycles((secs * 2e6) as u64);
    let f = m.frame();
    println!("seq {} {}x{} pc {:04X}", f.seq, f.active_w, f.active_h, m.cpu().pc);
    let mut pbm = format!("P4\n{} {}\n", f.active_line(0).len() * 8, f.active_h).into_bytes();
    pbm.extend_from_slice(&f.active_picture());
    std::fs::write(out, pbm).unwrap();
}
