#!/usr/bin/env bash
# 构造 MSVC + Windows SDK 环境并运行 cargo，绕过 Git Bash 的 link.exe 遮蔽。
# 用法: bash scripts/cargo-msvc.sh <cargo 参数...>

MSVC_ROOT="C:\\Program Files\\Microsoft Visual Studio\\18\\Community\\VC\\Tools\\MSVC\\14.50.35717"
SDK_ROOT="C:\\Program Files (x86)\\Windows Kits\\10"
SDK_VER="10.0.26100.0"

MSVC_BIN_POSIX="/c/Program Files/Microsoft Visual Studio/18/Community/VC/Tools/MSVC/14.50.35717/bin/Hostx64/x64"
# mt.exe（manifest tool）随 Windows SDK 分发。build.rs 里用 /MANIFESTINPUT 嵌入
# Common-Controls v6 manifest 时，链接器会去 PATH 找 mt.exe，找不到就报 LNK1158。
SDK_BIN_POSIX="/c/Program Files (x86)/Windows Kits/10/bin/$SDK_VER/x64"

export PATH="$MSVC_BIN_POSIX:$SDK_BIN_POSIX:/usr/bin:/bin"
export CARGO_BIN="${CARGO_BIN:-$HOME/.cargo/bin/cargo.exe}"

export INCLUDE="$MSVC_ROOT\\include;$SDK_ROOT\\Include\\$SDK_VER\\ucrt;$SDK_ROOT\\Include\\$SDK_VER\\um;$SDK_ROOT\\Include\\$SDK_VER\\shared;$SDK_ROOT\\Include\\$SDK_VER\\winrt"
export LIB="$MSVC_ROOT\\lib\\x64;$SDK_ROOT\\Lib\\$SDK_VER\\ucrt\\x64;$SDK_ROOT\\Lib\\$SDK_VER\\um\\x64"
export LIBPATH="$LIB"

exec "$CARGO_BIN" "$@"
