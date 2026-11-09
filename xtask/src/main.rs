use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
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
        Some("test") => test()?,
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
    build_hypervisor()?;
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

fn build_hypervisor() -> Result<(), DynError> {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let status = Command::new(&cargo)
        .current_dir(project_root().join("hypervisor"))
        .arg("build")
        .status()?;
    if !status.success() {
        return Err("hypervisor build failed.".into());
    }

    let src = project_root().join("target/x86_64-unknown-none/debug/hypervisor");
    let dst = project_root().join("esp/hypervisor.elf");
    fs::copy(&src, &dst).map_err(|e| {
        format!(
            "Failed to copy hypervisor from '{}' to '{}': {}",
            src.display(),
            dst.display(),
            e
        )
    })?;
    eprintln!("Copied {} -> {}", src.display(), dst.display());

    Ok(())
}

/// HV のログ（COM2）の出力先
const HV_LOG: &str = "hypervisor.log";
/// headless 実行時の管理 VM のコンソール（COM1）の出力先
const GUEST_LOG: &str = "guest.log";

/// 起動テストの成功判定に使う文字列（hypervisor/src/main.rs の BOOT_OK_MARKER と一致させる）
const BOOT_OK_MARKER: &str = "[hypervisor] boot ok";
/// 起動テストを即座に失敗させる文字列
const FAILURE_MARKERS: [&str; 2] = ["[PANIC]", "[EXCEPTION]"];
const DEFAULT_TEST_TIMEOUT_SECS: u64 = 60;

#[derive(Default)]
struct QemuOptions {
    debug: bool,
    trace: bool,
    vnc: bool,
    no_kvm: bool,
    smp: Option<u32>,
    iommu: bool,
}

impl QemuOptions {
    /// `run` / `test` 共通のオプションを解釈する。解釈できなかった引数は返す
    fn parse(args: &[String]) -> Result<(Self, Vec<String>), DynError> {
        let mut opts = QemuOptions::default();
        let mut rest = Vec::new();
        let mut iter = args.iter();
        while let Some(v) = iter.next() {
            match v.as_str() {
                "--debug" => opts.debug = true,
                "--trace" => opts.trace = true,
                "--novnc" => opts.vnc = true,
                "--no-kvm" => opts.no_kvm = true,
                "--iommu" => opts.iommu = true,
                "--smp" => {
                    let n = iter.next().ok_or("--smp requires a number.")?;
                    opts.smp = Some(
                        n.parse()
                            .map_err(|_| format!("Invalid --smp value: {}", n))?,
                    );
                }
                _ => rest.push(v.clone()),
            }
        }
        Ok((opts, rest))
    }

    /// KVM を使うかどうかを決める。使えない場合は理由を表示して TCG にする
    fn use_kvm(&self) -> bool {
        if self.no_kvm {
            return false;
        }
        if self.trace {
            eprintln!("note: --trace は TCG でしか動かないため、KVM を使わずに起動します");
            return false;
        }
        if !kvm_available() {
            eprintln!(
                "warning: /dev/kvm を開けないため TCG で起動します（ゲストから VMX は使えません）"
            );
            return false;
        }
        true
    }
}

fn kvm_available() -> bool {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_ok()
}

/// 環境変数で上書きできる OVMF のパス。未指定なら edk2 のビルド成果物を使う
fn ovmf_path(env_name: &str, file_name: &str) -> PathBuf {
    env::var_os(env_name).map(PathBuf::from).unwrap_or_else(|| {
        project_root()
            .join("edk2/Build/OvmfX64/DEBUG_GCC/FV")
            .join(file_name)
    })
}

/// OVMF_VARS.fd は QEMU が書き込む可能性があるため毎回新鮮なコピーを用意し、
/// 共通部分の引数を組み立てた QEMU コマンドを返す
fn qemu_command(opts: &QemuOptions) -> Result<Command, DynError> {
    let ovmf_code = ovmf_path("OVMF_CODE", "OVMF_CODE.fd");
    let ovmf_vars_src = ovmf_path("OVMF_VARS", "OVMF_VARS.fd");
    let ovmf_vars_dst = project_root().join("OVMF_VARS.fd");

    if !ovmf_code.exists() {
        return Err(format!(
            "OVMF_CODE.fd not found at '{}'. Build edk2 or set the OVMF_CODE environment variable.",
            ovmf_code.display()
        )
        .into());
    }
    fs::copy(&ovmf_vars_src, &ovmf_vars_dst).map_err(|e| {
        format!(
            "Failed to copy OVMF_VARS.fd from '{}' to '{}': {}",
            ovmf_vars_src.display(),
            ovmf_vars_dst.display(),
            e
        )
    })?;
    // 前回の実行のログが成功判定に混ざらないよう消しておく
    let _ = fs::remove_file(project_root().join(HV_LOG));

    let mut qemu_command = Command::new("qemu-system-x86_64");
    qemu_command.current_dir(project_root());

    if opts.use_kvm() {
        qemu_command.args(["-enable-kvm", "-cpu", "host,+vmx"]);
    } else {
        qemu_command.args(["-cpu", "max"]);
    }
    // intel-iommu を使うため q35 にする
    qemu_command.args(["-machine", "q35"]);
    if let Some(n) = opts.smp {
        qemu_command.args(["-smp", &n.to_string()]);
    }
    if opts.iommu {
        qemu_command.args(["-device", "intel-iommu"]);
    }

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
        "-no-reboot",
        "-debugcon",
        "file:debug.log",
        "-global",
        "isa-debugcon.iobase=0x402",
    ]);

    if opts.trace {
        qemu_command.args(["-D", "trace.log", "-d", "in_asm,int"]);
    }
    if opts.debug {
        qemu_command.args(["-s", "-S"]);
    }

    Ok(qemu_command)
}

/// COM2 の出力先。COM1 より後に指定すること（-serial は指定順に COM1, COM2 に割り当てられる）
fn hv_serial_args() -> [String; 4] {
    [
        "-chardev".into(),
        format!("file,id=hvlog,path={}", HV_LOG),
        "-serial".into(),
        "chardev:hvlog".into(),
    ]
}

fn run() -> Result<(), DynError> {
    let args: Vec<String> = env::args().skip(2).collect();
    let (opts, _) = QemuOptions::parse(&args)?;

    let mut qemu_command = qemu_command(&opts)?;
    if opts.vnc {
        qemu_command.args(["-vnc", "localhost:0"]);
    }
    // COM1: 管理 VM のコンソール（QEMU モニタと多重化）、COM2: HV のログ
    qemu_command.args(["-nographic", "-serial", "mon:stdio"]);
    qemu_command.args(hv_serial_args());

    let mut child = qemu_command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;

    // HV のログを "[hv] " を付けて端末にも流す
    let done = Arc::new(AtomicBool::new(false));
    let follower = {
        let done = done.clone();
        thread::spawn(move || follow_hv_log(&done))
    };

    let status = child.wait()?;
    done.store(true, Ordering::Release);
    let _ = follower.join();

    if !status.success() {
        return Err("QEMU exited with non-zero status.".into());
    }

    Ok(())
}

fn follow_hv_log(done: &AtomicBool) {
    let path = project_root().join(HV_LOG);
    let mut offset = 0;
    let mut pending = Vec::new();
    loop {
        // done を先に読み、最後に 1 回読み切ってから抜ける
        let finished = done.load(Ordering::Acquire);
        if let Ok(data) = fs::read(&path)
            && data.len() > offset
        {
            pending.extend_from_slice(&data[offset..]);
            offset = data.len();
            let mut out = std::io::stdout().lock();
            while let Some(pos) = pending.iter().position(|&b| b == b'\n') {
                let line = String::from_utf8_lossy(&pending[..pos]);
                // -nographic の端末は raw モードなので CR も出す
                let _ = write!(out, "[hv] {}\r\n", line.trim_end_matches('\r'));
                pending.drain(..=pos);
            }
            let _ = out.flush();
        }
        if finished {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// headless で起動し、HV のログに BOOT_OK_MARKER が出れば成功とする
fn test() -> Result<(), DynError> {
    let args: Vec<String> = env::args().skip(2).collect();
    let (opts, rest) = QemuOptions::parse(&args)?;

    let mut timeout = Duration::from_secs(DEFAULT_TEST_TIMEOUT_SECS);
    let mut iter = rest.iter();
    while let Some(v) = iter.next() {
        match v.as_str() {
            "--timeout" => {
                let secs = iter.next().ok_or("--timeout requires seconds.")?;
                timeout = Duration::from_secs(
                    secs.parse()
                        .map_err(|_| format!("Invalid --timeout value: {}", secs))?,
                );
            }
            _ => return Err(format!("Unknown option: {}", v).into()),
        }
    }

    let mut qemu_command = qemu_command(&opts)?;
    qemu_command.args([
        "-display",
        "none",
        "-monitor",
        "none",
        "-serial",
        &format!("file:{}", GUEST_LOG),
    ]);
    qemu_command.args(hv_serial_args());

    let mut child = qemu_command
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;

    let hv_log = project_root().join(HV_LOG);
    let start = Instant::now();
    let result = loop {
        let log = fs::read_to_string(&hv_log).unwrap_or_default();
        if log.contains(BOOT_OK_MARKER) {
            break Ok(());
        }
        if let Some(marker) = FAILURE_MARKERS.iter().find(|m| log.contains(*m)) {
            break Err(format!("HV reported {}", marker));
        }
        if let Some(status) = child.try_wait()? {
            break Err(format!("QEMU exited before boot completed ({})", status));
        }
        if start.elapsed() > timeout {
            break Err(format!("timed out after {}s", timeout.as_secs()));
        }
        thread::sleep(Duration::from_millis(100));
    };

    let _ = child.kill();
    let _ = child.wait();

    eprintln!("---- {} ----", HV_LOG);
    eprint!("{}", fs::read_to_string(&hv_log).unwrap_or_default());
    eprintln!("------------------------");

    match result {
        Ok(()) => {
            eprintln!("boot test passed ({:.1}s)", start.elapsed().as_secs_f64());
            Ok(())
        }
        Err(e) => Err(format!("boot test failed: {}", e).into()),
    }
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
  run [OPTION]       QEMU でブートローダーを起動する
                     COM1（管理 VM）は端末に、COM2（HV）は {hv_log} に出力し、端末にも [hv] 付きで表示する
  test [OPTION]      QEMU を headless で起動し、HV の起動完了を確認する（CI 用）
  help               このヘルプを表示する

Common Options (run / test):
  --no-kvm           /dev/kvm があっても TCG で起動する
  --smp <N>          CPU 数を指定する
  --iommu            intel-iommu を有効にする
  --debug            GDB サーバーとして起動する（ポート 1234、起動直後に停止）
  --trace            実行命令ログを trace.log に出力する（TCG で起動する）

run Options:
  --novnc            VNC サーバーを localhost:0 で起動する

test Options:
  --timeout <SECS>   起動完了を待つ秒数（デフォルト {timeout}）

Environment:
  OVMF_CODE / OVMF_VARS  OVMF のパスを上書きする（デフォルトは edk2 のビルド成果物）",
        hv_log = HV_LOG,
        timeout = DEFAULT_TEST_TIMEOUT_SECS,
    );
}
