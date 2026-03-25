use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::bootstrap::workspace_root;

const IMAGE_SUBSYSTEM_WINDOWS_GUI: u16 = 2;

#[test]
fn release_binary_uses_windows_gui_subsystem() {
    let status = Command::new(cargo_command())
        .args(["build", "-p", "astrcode-server", "--release"])
        .current_dir(workspace_root())
        .status()
        .expect("failed to build astrcode-server release binary");
    assert!(
        status.success(),
        "cargo build -p astrcode-server --release failed with status {status}"
    );

    let binary = workspace_root()
        .join("target")
        .join("release")
        .join("astrcode-server.exe");
    let subsystem = read_pe_subsystem(&binary);
    assert_eq!(
        subsystem,
        IMAGE_SUBSYSTEM_WINDOWS_GUI,
        "expected '{}' to use the Windows GUI subsystem so the Tauri sidecar does not spawn a terminal window",
        binary.display()
    );
}

fn cargo_command() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

fn read_pe_subsystem(path: &Path) -> u16 {
    let bytes = fs::read(path)
        .unwrap_or_else(|error| panic!("failed to read PE binary '{}': {error}", path.display()));
    assert!(
        bytes.len() >= 0x40,
        "PE binary '{}' is too small",
        path.display()
    );

    let pe_offset = u32::from_le_bytes(bytes[0x3C..0x40].try_into().unwrap()) as usize;
    let optional_header_offset = pe_offset + 4 + 20;
    let subsystem_offset = optional_header_offset + 68;
    assert!(
        subsystem_offset + 2 <= bytes.len(),
        "PE binary '{}' is truncated before subsystem field",
        path.display()
    );

    u16::from_le_bytes(
        bytes[subsystem_offset..subsystem_offset + 2]
            .try_into()
            .unwrap(),
    )
}
