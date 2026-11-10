# デバッグ・テスト環境

## 実行

```sh
cargo xtask build
cargo xtask run            # 対話モード
cargo xtask test           # headless の起動テスト（CI と同じ）
```

`cargo xtask help` でオプション一覧を表示できる。

### KVM と TCG

- `/dev/kvm` を開ければ `-enable-kvm -cpu host,+vmx` で起動する。HV が VMX を使うには KVM の nested 仮想化が必要
  - ホストで `cat /sys/module/kvm_intel/parameters/nested` が `Y` であること
  - TCG（ソフトウェアエミュレーション）は VMX をエミュレートしない
- `/dev/kvm` がない環境（コンテナ内など）では警告を出し、TCG（`-cpu max`）で起動する
  - HV は起動時に VMX のセルフテストを行うので、TCG では `failed to enable VMX: NotSupported` で panic する。TCG はブートローダや VMX を有効にする前の処理を調べるとき（`--trace` など）に使う
- `--trace`（`-d in_asm,int`）は TCG でしか動かないため、指定すると常に TCG で起動する
- `--no-kvm` で明示的に TCG を使える

### シリアルの割り当て

| ポート | 用途 | `run` | `test` |
|---|---|---|---|
| COM1 (0x3f8) | 管理 VM（Linux）のコンソール | 端末（QEMU モニタと多重化。`Ctrl-a c` で切り替え） | `guest.log` |
| COM2 (0x2f8) | HV のログ | `hypervisor.log`（端末にも `[hv]` を付けて表示） | `hypervisor.log` |
| debugcon (0x402) | OVMF のデバッグログ | `debug.log` | `debug.log` |

OVMF は ConOut をすべてのシリアルに出すので、ブートローダの出力は COM1 と COM2 の両方に現れる。

### 起動テスト（`cargo xtask test`）

- HV は VMX を有効にし、`vmx/selftest.rs` で次を確認してから `[hypervisor] boot ok` を出す
  - 管理 VM 用の EPT で HV のメモリがマップされておらず、それ以外が恒等マップされていること
  - 最小のテストゲストで CPUID / VMCALL / CR4 / XSETBV の exit 処理、EPT の遅延マップ、HV のヒープへのアクセスが EPT violation で止められること（このときゲスト状態のダンプが出るが、想定どおりの動作）
- HV のログに `[hypervisor] boot ok` が出れば成功
- `[PANIC]` か `[EXCEPTION]` が出るか、タイムアウト（デフォルト 60 秒、`--timeout` で変更）になれば失敗
- `--allow-no-vmx`: CPU が VMX に対応していない場合、HV が `[hypervisor] VMX is not available on this CPU` を出した時点で成功とする
  - GitHub Actions のランナーは KVM は使えるが nested VMX は使えないため、CI ではこれを付けて VMX の手前までを確認している
  - **VMX 以降の動作は CI では確認されない**。VMX に関わる変更は、nested KVM が使える環境で `cargo xtask test`（`--allow-no-vmx` なし）を実行して確認すること
- 管理 VM が起動するようになったら、判定条件を Linux のプロンプトに変更する（#6）

### OVMF

デフォルトでは `edk2/Build/OvmfX64/DEBUG_GCC/FV/` を使う。環境変数 `OVMF_CODE` / `OVMF_VARS` で上書きできる（CI では apt の `ovmf` パッケージを使っている）。

## HV の異常時の出力

- **panic**: `[PANIC] <メッセージ>` を出して停止する
- **CPU 例外**: `[EXCEPTION]` に続けてベクタ、エラーコード、汎用レジスタ、CR0/CR2/CR3/CR4 を出して停止する
  - #DF と NMI は IST の専用スタックで処理するので、スタックオーバーフローでもダンプが出る
- **VMX**: VMLAUNCH / VMRESUME の失敗、VM-entry failure、処理できない exit では `[vmx]` に続けて exit reason とゲスト状態（RIP / RSP / CR / セグメント / 汎用レジスタ）をダンプする

## GDB

```sh
cargo xtask run --debug    # ポート 1234 で待ち受け、起動直後に停止する
gdb -x test.gdb            # 別の端末で
```

- HV の ELF は上位アドレス（`0xffffffff8xxxxxxx`）にリンクされている。`boot.S` がページテーブルを切り替えて `main` を呼んだ後は、シンボルを読み込めばそのまま使える

  ```
  symbol-file target/x86_64-unknown-none/debug/hypervisor
  hbreak main
  ```

- `boot.S` の先頭（ページテーブル切り替え前）で止めたいときは、物理アドレスに `hbreak` を置く。ロード先の物理アドレスは、ブートローダが出す `addr: 0x...` で確認する
- KVM 下では、ページテーブルを切り替える前後で止める場合はソフトウェアブレークポイントより `hbreak` のほうが確実
