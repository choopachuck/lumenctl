//! Prints every detected monitor with its current DDC/CI values (read-only).

use lumen_core::backend::default_backend;
use lumen_core::vcp;

fn main() -> anyhow::Result<()> {
    for mut d in default_backend().enumerate()? {
        let i = &d.info;
        println!(
            "#{} {} [{}] {} {:?} serial={:?} output={:?} rect={:?}",
            i.index, i.name, i.id, i.gdi_device, i.manufacturer_id, i.serial, i.output, i.rect
        );
        for &code in vcp::POLLED {
            match d.device.get_vcp(code) {
                Ok(v) => println!("    0x{code:02X} {:<24} {} / {}", vcp::vcp_name(code), v.current, v.max),
                Err(e) => println!("    0x{code:02X} {:<24} error: {e}", vcp::vcp_name(code)),
            }
        }
        match d.device.capabilities() {
            Ok(c) => println!("    caps: {c}"),
            Err(e) => println!("    caps error: {e}"),
        }
    }
    Ok(())
}
