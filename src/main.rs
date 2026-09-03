use ovmf_prebuilt::{Arch, FileType, Prebuilt, Source};
use std::env;
use std::process::{Command, exit};

fn main() {
    // read env variables that were set in build script
    let uefi_path = env!("UEFI_PATH");
    let bios_path = env!("BIOS_PATH");

    // parse mode from CLI
    let args: Vec<String> = env::args().collect();
    let prog = &args[0];

    let mut debug = false;
    let mut uefi = None;

    for arg in &args[1..] {
        match arg.to_lowercase().as_str() {
            "uefi" => uefi = Some(true),
            "bios" => uefi = Some(false),
            "--debug" => debug = true,
            "-h" | "--help" => {
                println!("Usage: {prog} [uefi|bios] [--debug]");
                println!("  uefi      - boot using OVMF (UEFI)");
                println!("  bios      - boot using legacy BIOS");
                println!("  --debug   - enable QEMU debug logging and halt on triple fault");
                exit(0);
            }
            other => {
                eprintln!("Unknown argument: {other}");
                eprintln!("Usage: {prog} [uefi|bios] [--debug]");
                exit(1);
            }
        }
    }

    let uefi = match uefi {
        Some(u) => u,
        None => {
            eprintln!("Usage: {prog} [uefi|bios] [--debug]");
            exit(1);
        }
    };

    let mut cmd = Command::new("qemu-system-x86_64");
    // print serial output to the shell
    cmd.arg("-serial").arg("mon:stdio");
    // don't display video output
    // cmd.arg("-display").arg("none");
    // enable the guest to exit qemu
    cmd.arg("-device")
        .arg("isa-debug-exit,iobase=0xf4,iosize=0x04");

    if uefi {
        let prebuilt =
            Prebuilt::fetch(Source::LATEST, "target/ovmf").expect("failed to update prebuilt");

        let code = prebuilt.get_file(Arch::X64, FileType::Code);
        let vars = prebuilt.get_file(Arch::X64, FileType::Vars);

        cmd.arg("-drive")
            .arg(format!("format=raw,file={uefi_path}"));
        cmd.arg("-drive").arg(format!(
            "if=pflash,format=raw,unit=0,file={},readonly=on",
            code.display()
        ));
        // copy vars and enable rw instead of snapshot if you want to store data (e.g. enroll secure boot keys)
        cmd.arg("-drive").arg(format!(
            "if=pflash,format=raw,unit=1,file={},snapshot=on",
            vars.display()
        ));
    } else {
        cmd.arg("-drive")
            .arg(format!("format=raw,file={bios_path}"));
    }

    if debug {
        cmd.arg("-d").arg("int,cpu_reset,guest_errors,unimp");
        cmd.arg("-D").arg("/tmp/qemu.log");
        cmd.arg("-no-reboot");
        eprintln!("[debug] QEMU debug logging to /tmp/qemu.log");
        eprintln!("[debug] Halting on triple fault instead of rebooting");
    }

    let mut child = cmd.spawn().expect("failed to start qemu-system-x86_64");
    let status = child.wait().expect("failed to wait on qemu");
    match status.code().unwrap_or(1) {
        0x10 => 0, // success
        0x11 => 1, // failure
        _ => 2,    // unknown fault
    };
}
