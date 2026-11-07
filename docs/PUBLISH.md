# GitHub 发布准备

计划账号：`yanhuigang173`。本地元信息暂按仓库名 `QuotaDock` 整理；这不表示远端仓库已经存在。

## 当前整理范围

- 当前 Rust 原生源码、锁文件、构建与验证脚本。
- README、使用与构建文档、主题预览、更新记录、MIT 许可及上游署名。
- GitHub 问题反馈与 Pull Request 模板。
- 历史 JavaScript / Tauri 源码保留，并明确标记为非当前构建。
- 不包含 EXE、Cargo 缓存、个人 Codex 文件、测试会话、诊断记录和本地参考图。

## 导出源码

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/prepare-github.ps1
```

在 `out/github/` 生成独立源码目录和 ZIP。脚本按明确的目录 / 文件列表复制，不改变原项目，不删除历史文件，也不连接 GitHub。

## 后续上传

确认仓库名和公开范围后，在 `yanhuigang173` 下创建空仓库，再进入导出的源码目录：

```powershell
git init -b main
git add .
git commit -m "Initial QuotaDock source release"
git remote add origin https://github.com/yanhuigang173/QuotaDock.git
git push -u origin main
```

上述是后续操作说明，本次整理不会执行推送。若仓库不是空仓库，应先读取远端历史并决定如何合并，不直接覆盖。

## 发布可执行文件

使用原生构建脚本生成 EXE，在 GitHub Release 附加 `QuotaDock.exe` 和相应校验值，不把二进制提交进源码目录。发布前检查版本号在 `package.json`、`src-native/Cargo.toml`、`src-native/resource.rc` 中保持一致，并完成对应验证。

当前整理未配置自动打包或自动发布工作流。
