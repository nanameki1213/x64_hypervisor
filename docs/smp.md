# マルチプロセッサ

vCPU と物理 CPU は 1:1 で固定し、HV はスケジューラを持たない。HV はタイマ割り込みを使わず、HV に制御が来るのは VM exit のときだけ。

## 起動の流れ

1. **CPU の列挙**（`arch/x86_64/smp.rs` の `discover`）: MADT の Local APIC / x2APIC エントリから、使える CPU の APIC ID を集める
2. **AP の起動**（`start_aps`、VMX より前）
   - 1MiB 未満の空きページにトランポリン（リアルモード → 64bit モード）を置き、AP を 1 つずつ INIT-SIPI-SIPI で起こす。待ち時間は ACPI PM タイマで測る
   - AP は HV のページテーブルを共有し、`ap_entry` で自分専用の GDT / TSS / IST スタック（`gdt::CpuTables`）と共有の IDT を読み込む
3. **AP の VMX**: 各 AP は VMXON し、BSP が管理 VM の EPT を作るのを待ってから、管理 VM の vCPU として **wait-for-SIPI** の activity state でゲストに入る
4. BSP はすべての AP がゲストに入ったのを待ってから、自分の vCPU で Linux を起動する
5. Linux が AP に送った INIT / SIPI は、物理的には VMX non-root で待っている AP に届き、VM exit になる
   - **SIPI**: ベクタ VV から CS = VV00、RIP = 0 を設定して実行を始める（`[vmx] SIPI (vector ...) on APIC ID ...` を出す）
   - **INIT**: vCPU を INIT 直後の状態（リアルモード）に戻して、再び SIPI を待つ

AP のゲストはリアルモードから始まるので、unrestricted guest が必要（[memory.md](memory.md)）。

## 共有するもの・CPU ごとのもの

| | 共有 | CPU ごと |
|---|---|---|
| HV | ページテーブル、IDT、ヒープ、コンソール（spin lock） | GDT / TSS / IST スタック、スタック（AP は 64KiB）、VMXON 領域、VMCS |
| 管理 VM | EPT（`&'static Mutex<Ept>`） | vCPU（VMCS と汎用レジスタ） |

exit ハンドラは対象の `Vcpu` を引数で受け取るので、GS base などで引く per-CPU データは今のところ使っていない。

## 制限・今後

- AP の起動に ACPI PM タイマが必要（FADT の `PM_TMR_BLK`）
- トランポリンを使うため、HV のページテーブル（`boot_pml4`）は 4GiB 未満になければならない
- HV 内部の CPU 間通信（INVEPT shootdown など）はまだない。EPT の遅延マップは存在しなかったエントリを足すだけなので不要
- フェーズ 2 で CPU をユーザ VM に分けるときは、管理 VM に見せる MADT から、その CPU を除く必要がある
