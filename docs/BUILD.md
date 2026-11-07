# 构建说明

## Windows 环境

当前原生项目的路径是 `src-native/`，版本为 0.4.0。Node.js 仅用于历史 JavaScript 工具，不是原生构建的前置条件。

1. 安装 Rust / rustup。
2. 安装 MinGW-w64 工具链，确保同一架构的 `gcc.exe`、`windres.exe` 在 PATH 中。使用 MSYS2 时，可使用其 UCRT64 或 MINGW64 工具链；不要混用工具链目录。
3. 在项目根目录运行：

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup override set stable-x86_64-pc-windows-gnu
gcc --version
windres --version
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1
```

首次构建需要联网解析锁文件中的依赖；之后可使用 `scripts/build.ps1 -Offline`。应保留 `src-native/Cargo.lock`，避免构建时隐式升级依赖。

构建脚本生成 `dist/QuotaDock.exe`，并关闭同一路径的旧运行实例，以便替换文件；不会自动重新启动。

```powershell
Start-Process -FilePath .\dist\QuotaDock.exe -WindowStyle Hidden
```

`src-native/build.rs` 在 GNU 目标下调用 `windres` 嵌入图标和版本信息。当前发布流程以 Windows x64 GNU 为准；直接使用其他目标工具链不等于获得相同的图标和资源信息。

## 开发与验证

```powershell
cargo run --manifest-path src-native/Cargo.toml
cargo test --manifest-path src-native/Cargo.toml --locked
cargo clippy --manifest-path src-native/Cargo.toml --all-targets --locked
```

交互验证需要桌面会话，并可能临时显示隔离测试窗口：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-task-notifications.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-desktop-notifications.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-fusion.ps1
```

测试记录位于 `artifacts/`，不要提交。不要将本机的 Codex 凭据或真实会话复制进测试数据。

生成预览：

```powershell
cargo run --manifest-path src-native/Cargo.toml --example render_previews --release --locked
```

输出目录 `assets/ui-previews/` 已忽略；README 使用的少量精选预览位于 `docs/images/`。
