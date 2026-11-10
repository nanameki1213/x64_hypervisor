# 管理 VM（Linux）

`cargo xtask linux` が、このディレクトリのファイルから管理 VM のカーネルと initramfs を作り、ESP に置く。

| ファイル | 用途 | ESP 上の名前 |
|---|---|---|
| `mgmt-vm.config` | カーネルの設定（`allnoconfig` に重ねる） | （`bzImage` を生成） |
| `init` | initramfs の `/init`（busybox のシェルを起動する） | （`initrd.img` を生成） |
| `cmdline.txt` | カーネルのコマンドライン | `cmdline.txt` |

詳細は [docs/linux-boot.md](../docs/linux-boot.md) を参照。
