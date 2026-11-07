# 平台建置驗證

- Windows x86_64：本機建置、測試、打包皆已驗證（最近：2026-11-07，`af2c75e`）。
- Linux x86_64：本機環境沒有工具鏈，未做原生建置；由 GitHub Actions `ubuntu-latest` 建置與測試。曾修正 Linux CI 的 dead_code 錯誤（`5575822`）。
- macOS：未建置、未驗證。
