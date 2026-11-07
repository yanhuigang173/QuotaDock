# 来源与第三方说明

QuotaDock 在已有 Codex 额度小组件基础上演进，并以 Rust / Win32 重写当前运行界面。

## 上游项目

- [xicunwus2025-sys/codex-led-widget](https://github.com/xicunwus2025-sys/codex-led-widget)：原始 Windows Codex 额度组件。
- [stupdada/codex-quota-widget](https://github.com/stupdada/codex-quota-widget)：本地工作目录此前元信息所指向的项目。

两个上游项目的 `package.json` 均声明 MIT，作者字段为 `Codex Quota Widget Contributors`。本项目保留对应署名；该署名不表示上游作者维护或认可本分支。

历史 JavaScript 和 Tauri 实现分别保留在 `src/` 和 `src-tauri/`，不参与当前原生 EXE 构建。

## 依赖与字体

Rust 依赖及精确版本见 `src-native/Cargo.toml` 和 `src-native/Cargo.lock`，依赖各自的许可证仍然适用。

程序调用本机已安装的字体，不捆绑 Microsoft 字体或 Noto 字体文件；没有思源黑体时会回退到系统字体。

QuotaDock 是独立项目，与 OpenAI 没有官方关联。Codex 等名称归各自权利人所有。
