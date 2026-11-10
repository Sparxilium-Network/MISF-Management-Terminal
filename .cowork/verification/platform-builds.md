# 平台建置驗證

- Windows x86_64：Slint＋Skia 版本機建置、測試、打包皆已驗證（最近：2026-11-10，`f884c7a`）；GitHub Actions `windows-latest` 也通過（run 42822716899，`3f7942d`）。
- Linux x86_64：本機環境沒有工具鏈，未做原生建置。Slint＋Skia 版成為 `main` 後，GitHub Actions `ubuntu-latest` 已通過 fmt、Clippy、測試（28、20、55、24 項）、release 建置與打包，並發布 Pre-release `dev-22.1-3f7942d`（run 42822716899，2026-11-10）；Linux 版 Skia 預編譯檔下載正常。未在 Linux 桌面實際執行 app。Vulkan Memory Allocator 授權依 `vulkan` 功能與 Skia 預設推論，未以 Linux 版 Skia 符號確認。
- macOS：未建置、未驗證。
