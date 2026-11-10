# 平台建置驗證

- Windows x86_64：Slint＋Skia 版（分支 `slint-rego`）本機建置、測試、打包皆已驗證（最近：2026-11-10，`f6bbbd4`）。
- Linux x86_64：本機環境沒有工具鏈，未做原生建置。egui 版由 GitHub Actions `ubuntu-latest` 建置與測試過（曾修正 Linux CI 的 dead_code 錯誤，`5575822`）；Slint＋Skia 版只存在本機分支、未推送，CI 不會跑，因此 Linux 建置、Linux 版 Skia 預編譯檔下載與 Vulkan Memory Allocator 授權是否正確皆未驗證。
- macOS：未建置、未驗證。
