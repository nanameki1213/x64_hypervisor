use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

type DynError = Box<dyn std::error::Error>;

fn main() {
    if let Err(e) = try_main() {
        eprintln!("{}", e);
        std::process::exit(-1);
    }
}

fn try_main() -> Result<(), DynError> {
    let task = env::args().nth(1);
    match task.as_deref() {
        Some("build") => build()?,
        Some("run") => run()?,
        Some("help") | Some("-h") | Some("--help") => print_help(),
        _ => {
            print_help();
            return Err("Unknown or missing command.".into());
        }
    }
    Ok(())
}

fn build() -> Result<(), DynError> {
    build_bootloader()?;
    build_kernel()?;
    Ok(())
}

fn build_bootloader() -> Result<(), DynError> {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let status = Command::new(&cargo)
        .current_dir(project_root().join("bootloader"))
        .arg("build")
        .status()?;
    if !status.success() {
        return Err("bootloader build failed.".into());
    }

    let src = project_root().join("target/x86_64-unknown-uefi/debug/bootloader.efi");
    let dst = project_root().join("esp/EFI/BOOT/BOOTX64.EFI");
    fs::create_dir_all(dst.parent().unwrap())?;
    fs::copy(&src, &dst).map_err(|e| {
        format!(
            "Failed to copy bootloader from '{}' to '{}': {}",
            src.display(),
            dst.display(),
            e
        )
    })?;
    eprintln!("Copied {} -> {}", src.display(), dst.display());

    Ok(())
}

fn build_kernel() -> Result<(), DynError> {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let status = Command::new(&cargo)
        .current_dir(project_root().join("kernel"))
        .arg("build")
        .status()?;
    if !status.success() {
        return Err("kernel build failed.".into());
    }

    let src = project_root().join("target/x86_64-unknown-none/debug/kernel");
    let dst = project_root().join("esp/kernel.elf");
    fs::copy(&src, &dst).map_err(|e| {
        format!(
            "Failed to copy kernel from '{}' to '{}': {}",
            src.display(),
            dst.display(),
            e
        )
    })?;
    eprintln!("Copied {} -> {}", src.display(), dst.display());

    Ok(())
}

fn run() -> Result<(), DynError> {
    let mut is_debug = false;
    let mut is_trace = false;

    let args: Vec<String> = env::args().collect();
    let mut args_iter = args.iter().skip(2);

    while let Some(v) = args_iter.next() {
        if v == "--debug" {
            is_debug = true;
        } else if v == "--trace" {
            is_trace = true;
        }
    }

    let ovmf_vars_src = project_root().join("edk2/Build/OvmfX64/DEBUG_GCC/FV/OVMF_VARS.fd");
    let ovmf_vars_dst = project_root().join("OVMF_VARS.fd");
    let ovmf_code = project_root().join("edk2/Build/OvmfX64/DEBUG_GCC/FV/OVMF_CODE.fd");

    // OVMF_VARS.fd は QEMU が書き込む可能性があるため毎回新鮮なコピーを用意する
    fs::copy(&ovmf_vars_src, &ovmf_vars_dst).map_err(|e| {
        format!(
            "Failed to copy OVMF_VARS.fd from '{}' to '{}': {}",
            ovmf_vars_src.display(),
            ovmf_vars_dst.display(),
            e
        )
    })?;

    let mut qemu_command = Command::new("qemu-system-x86_64");
    qemu_command.current_dir(project_root());
    qemu_command.args([
        "-drive",
        &format!(
            "if=pflash,format=raw,readonly=on,file={}",
            ovmf_code.display()
        ),
        "-drive",
        &format!("if=pflash,format=raw,file={}", ovmf_vars_dst.display()),
        "-drive",
        "format=raw,file=fat:rw:esp",
        "-net",
        "none",
        "-serial",
        "mon:stdio",
        "-debugcon",
        "file:debug.log",
        "-global",
        "isa-debugcon.iobase=0x402",
    ]);

    if is_trace {
        qemu_command.args(["-D", "trace.log", "-d", "in_asm,int"]);
    }
    if is_debug {
        qemu_command.args(["-s", "-S"]);
    }

    let status = qemu_command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    if !status.success() {
        return Err("QEMU exited with non-zero status.".into());
    }

    Ok(())
}

fn project_root() -> PathBuf {
    Path::new(&env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(1)
        .unwrap()
        .to_path_buf()
}

fn print_help() {
    eprintln!(
        "Usage: cargo xtask <command>

Commands:
  build              ブートローダーとカーネルをビルドして esp/ に配置する
  run [OPTION]       OVMF_VARS.fd をコピーし QEMU でブートローダーを起動する
  help               このヘルプを表示する

run Options:
  --debug            GDB サーバーとして起動する（ポート 1234、起動直後に停止）
  --trace            実行命令ログを trace.log に出力する"
    );
}
