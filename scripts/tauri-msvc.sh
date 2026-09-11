#!/usr/bin/env bash
# 在 MSVC + Windows SDK + cargo 齐备的环境下运行 `npm run tauri`。
#
# 为什么需要它：
# 1) 本机 PATH 里没有 cargo（平时通过 scripts/cargo-msvc.sh 调），Tauri CLI 启动时会
#    立刻执行 `cargo metadata`，找不到就报 "program not found"。
# 2) cargo-msvc.sh 为了绕开 Git Bash 的 /usr/bin/link.exe 遮蔽，把 PATH 压成了
#    "MSVC;SDK;/usr/bin;/bin"，node/npm 会一起消失，所以不能直接 source 它。
# 3) MSVC 的 bin 必须排在 /usr/bin 之前，否则链接器又会被 link.exe 遮蔽。
#
# 用法: bash scripts/tauri-msvc.sh build --no-bundle
#       bash scripts/tauri-msvc.sh build            # 需要本机装了 NSIS/WiX
#       bash scripts/tauri-msvc.sh dev

set -euo pipefail
cd "$(dirname "$0")/.."

MSVC_ROOT="C:\\Program Files\\Microsoft Visual Studio\\18\\Community\\VC\\Tools\\MSVC\\14.50.35717"
SDK_ROOT="C:\\Program Files (x86)\\Windows Kits\\10"
SDK_VER="10.0.26100.0"

MSVC_BIN_POSIX="/c/Program Files/Microsoft Visual Studio/18/Community/VC/Tools/MSVC/14.50.35717/bin/Hostx64/x64"
# mt.exe（manifest tool）随 SDK 分发。build.rs 用 /MANIFESTINPUT 嵌入 Common-Controls v6
# manifest 时链接器会去 PATH 找 mt.exe，找不到报 LNK1158。
SDK_BIN_POSIX="/c/Program Files (x86)/Windows Kits/10/bin/$SDK_VER/x64"

export PATH="$MSVC_BIN_POSIX:$SDK_BIN_POSIX:$HOME/.cargo/bin:$PATH"
export INCLUDE="$MSVC_ROOT\\include;$SDK_ROOT\\Include\\$SDK_VER\\ucrt;$SDK_ROOT\\Include\\$SDK_VER\\um;$SDK_ROOT\\Include\\$SDK_VER\\shared;$SDK_ROOT\\Include\\$SDK_VER\\winrt"
export LIB="$MSVC_ROOT\\lib\\x64;$SDK_ROOT\\Lib\\$SDK_VER\\ucrt\\x64;$SDK_ROOT\\Lib\\$SDK_VER\\um\\x64"
export LIBPATH="$LIB"

exec npm run tauri -- "$@"
