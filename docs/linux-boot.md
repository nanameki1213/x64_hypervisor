# 管理 VM（Linux）の起動

HV は起動後、管理 VM として Linux を 1 つ起動する（フェーズ 1）。デバイスは管理 VM にパススルーし、HV はハードウェアを管理しない。

## 使い方

```sh
cargo xtask build          # ブートローダと HV
cargo xtask linux          # カーネルと initramfs（初回はソースの取得とビルドで数分）
cargo xtask run            # 端末に Linux のコンソールが出て、busybox のシェルが使える
cargo xtask test           # 管理 VM のシェルの起動まで確認する
```

- ESP に `bzImage` がなければ、HV はセルフテストの後に停止する（CI はこの状態で動く）
- `/workspace` のようなネットワークファイルシステム上ではソースの展開とビルドが遅い。`XTASK_LINUX_DIR=/path/on/local/disk` でビルド先を変えられる
- initramfs には静的リンクの busybox が必要（Ubuntu なら `busybox-static`）。`BUSYBOX` でパスを指定できる
- カーネルのビルドに必要なもの: `make`、`gcc`、`flex`、`bison`、`bc`、`libelf-dev`、`curl`、`xz`

## 全体の流れ

1. **ブートローダ**（`bootloader/src/main.rs`）が、`ExitBootServices` の前に ESP のルートから次のファイルを 4GiB 未満の LoaderData に読み込み、`BootInfo`（`support/boot_info`）で HV に渡す
   - `hypervisor.elf`（必須）、`bzImage`、`initrd.img`、`cmdline.txt`（なくてもよい）
   - ACPI の RSDP のアドレス
2. **HV**（`hypervisor/src/linux/`）が、VMX のセルフテストの後に管理 VM の初期状態を作る
   - カーネルの配置、boot_params（zero page）、e820、ゲスト用の GDT とページテーブル
3. 管理 VM 用の EPT（HV のメモリ以外の恒等マップ、[memory.md](memory.md)）で vCPU を作り、64bit エントリ（`code32_start + 0x200`）から実行する
4. AP は HV の起動時に起こしてあり、管理 VM の vCPU として SIPI を待っている。Linux が AP を起こすと、その CPU で実行が始まる（[smp.md](smp.md)）

## ブートプロトコル

Linux x86 boot protocol（`Documentation/arch/x86/boot.rst`）の **64bit エントリ** を使う。プロトコル 2.12 以上（`XLF_KERNEL_64`）が必要。

### メモリの配置

置き場所は、UEFI のメモリマップのうち `ExitBootServices` の後に空いている領域（ConventionalMemory、BootServicesCode / Data）から、HV のメモリを除いて選ぶ。bzImage などのファイルはブートローダが LoaderData に置いたので、この候補には入らない。

| 内容 | 場所 |
|---|---|
| カーネル本体（bzImage の setup 以降） | `pref_address`（通常 16MiB）から `init_size` バイト。空いていなければ、`kernel_alignment` にアラインした空き領域（`relocatable_kernel` のとき） |
| 起動データ（64KiB） | 1MiB〜4GiB の空き領域のうち最も低いところ |
| initrd | ブートローダが読み込んだ場所のまま（`initrd_addr_max` 以下であることを確認する） |

起動データの中身:

| オフセット | 内容 |
|---|---|
| `0x0000` | boot_params（zero page）。RSI でカーネルに渡す |
| `0x1000` | cmdline（NUL 終端） |
| `0x2000` | GDT（`0x10` = 64bit コード、`0x18` = データ。boot protocol の `__BOOT_CS` / `__BOOT_DS`） |
| `0x3000`〜 | ページテーブル。下位 4GiB を 2MiB ページで恒等マップする（PML4、PDPT、PD × 4） |
| 〜`0xd000` | スタック |

カーネルは起動直後に自分の GDT とページテーブルに切り替え、boot_params と cmdline をコピーするので、起動データはその後は使われない（e820 では usable のままにしている）。

### boot_params

- bzImage の setup header をそのままコピーし、ブートローダが書くフィールドだけを埋める
  - `type_of_loader = 0xff`（未登録のブートローダ）、`loadflags |= LOADED_HIGH`、`code32_start`
  - `cmd_line_ptr` / `ext_cmd_line_ptr`、`ramdisk_image` / `ramdisk_size`（と `ext_*`）
  - `acpi_rsdp_addr`: EFI を使わない起動なので、ここで教えないと Linux は RSDP を見つけられない
- **e820**: UEFI のメモリマップを Linux の EFI stub と同じ対応で変換し、隣接する同じタイプのエントリをまとめる
  - **HV のメモリ（イメージとヒープ）は reserved にする**。穴（エントリなし）にすると、Linux がそこを PCI の MMIO の割り当て先に使うことがあるため
  - zero page に入るのは 128 エントリまで。超えたら起動を中止する（TODO: `SETUP_E820_EXT`）
- `efi_info` は渡さない。Linux は EFI を使わずに起動する（`EFI runtime services` は使えない。RTC は CMOS で読める）

### CPU の初期状態

- 64bit モード、ページング有効（CR0 = PE | ET | NE | PG、CR4 = PAE、EFER = LME | LMA）
- CS = `0x10`、DS / ES / SS / FS / GS = `0x18`、割り込み禁止、IDT は空
- RIP = カーネルのロード先 + `0x200`、RSI = boot_params、RSP = 起動データ内のスタック
- HLT exiting は無効（管理 VM の HLT は物理 CPU をそのまま止める）

## カーネルと initramfs

`linux/` のファイルから作る（`cargo xtask linux`、`xtask/src/linux.rs`）。

- **カーネル**: kernel.org の longterm（`xtask/src/linux.rs` の `LINUX_VERSION`、SHA-256 で検証）
  - `make allnoconfig` に `linux/mgmt-vm.config` を重ねる。依存関係が満たされずに無視された設定があればエラーにする
  - シリアルコンソール、initramfs、ACPI、PCI だけを有効にした最小構成。KASLR は使わない
- **initramfs**: 無圧縮の cpio（newc）を xtask が直接書き出す。中身は busybox、`/dev/console`、`/init`（`linux/init`）
  - `/init` は proc / sysfs / devtmpfs をマウントし、`[guest] initramfs ok` を出してシェルを起動する。起動テストはこの行を成功判定に使う
- **cmdline**（`linux/cmdline.txt`）
  - `console=uart8250,io,0x3f8,115200n8`: コンソールを I/O アドレスで指定する。ACPI の列挙順では COM2 が ttyS0 になるため、`console=ttyS0` では HV のログ（COM2）にコンソールが出てしまう

## 今のところの制限・課題

- **COM2 が管理 VM から見えている**。Linux は COM2 を ttyS1 として初期化するので、実機では HV のログと衝突しうる（QEMU のファイル出力では問題にならない）。I/O ビットマップと ACPI での隠蔽は #10 で扱う
- ACPI テーブルは書き換えずにそのまま渡している（#10）
- VMX の制御で有効にできなかった命令（RDTSCP / RDPID、INVPCID、XSAVES、WAITPKG）は CPUID でも隠している（`exit::handle_cpuid`）。隠さないと Linux がそれを使って #UD になる
