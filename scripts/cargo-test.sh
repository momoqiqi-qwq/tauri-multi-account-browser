#!/usr/bin/env bash
# 运行 Rust 单元测试。
#
# 为什么不能直接用 `cargo test`：
#   dev-dependencies 打开了 tauri 的 `test` feature，测试二进制会链入 tray-icon/muda，
#   它们引用 comctl32.dll!TaskDialogIndirect。该导出只在 Common-Controls **v6**
#   （WinSxS 旁加载程序集）里存在，而 System32\comctl32.dll 是 v5.82，并不导出它。
#   没有 v6 manifest，加载器会绑到 v5.82，进程启动即失败：
#     STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139)
#
#   正式 app 由 tauri-build 注入 manifest，所以只有测试目标需要补。
#   这里用 `--config` 临时注入 rustflags —— 只作用于本次调用，
#   不会污染 `cargo build --release`（否则 app 会出现两份 MANIFEST 资源，
#   链接时报 CVT1100 资源重复）。
#
# 用法: bash scripts/cargo-test.sh [额外 cargo 参数...]
#   例: bash scripts/cargo-test.sh --lib

set -euo pipefail

# 定位到仓库根目录（本脚本在 scripts/ 下），再进入 src-tauri —— cargo 需要在那儿运行。
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT/src-tauri"

if [ ! -f "common-controls.manifest" ]; then
  echo "错误：找不到 src-tauri/common-controls.manifest" >&2
  exit 1
fi

# 转成 Windows 反斜杠路径；link.exe / mt.exe 只认这种形式。
MANIFEST_WIN="$(cygpath -w "$PWD/common-controls.manifest" 2>/dev/null || echo "$PWD/common-controls.manifest")"

# TOML 字面量字符串（单引号）里的反斜杠不转义，正好可以直接写 Windows 路径。
CONFIG="target.x86_64-pc-windows-msvc.rustflags=[ '-C', 'link-arg=/MANIFEST:EMBED', '-C', 'link-arg=/MANIFESTINPUT:${MANIFEST_WIN}' ]"

# tauri-plugin-store 的 store 按文件路径在进程内共享，多个 mock app 拿到同一份数据；
# 并行跑会互相踩到对方预置的数据，因此强制单线程。
export RUST_TEST_THREADS=1

# 复用 cargo-msvc.sh 里的 MSVC/Windows SDK 环境（mt.exe 必须在 PATH 上，否则 LNK1158）。
exec bash ../scripts/cargo-msvc.sh test --config "$CONFIG" "$@"
