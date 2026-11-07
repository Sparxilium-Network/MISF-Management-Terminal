# 2026-10-30 依賴更新

- 來源：crates.io 官方 API、Cargo registry、RustSec（Asia/Taipei 當日）。
- 應用程式及本機 FIDO2 副本的直接 registry 依賴對齊當日最新穩定版，共核對 27 種直接 registry 套件（含圖示建置套件）。未強制覆蓋上游不相容的間接依賴，因此不能宣稱依賴樹每個節點都是最新版。
- 應用程式的 `rand_core 0.6` OS 亂數呼叫改為 `getrandom 0.4.3`。UI／SSH／容器功能沒有因更新而停用。

| 依賴 | 更新後版本 |
| --- | --- |
| eframe | 0.37.0 |
| rfd | 0.17.2 |
| tokio | 1.54.1 |
| serde / serde_json | 1.0.229 / 1.0.154 |
| chrono / uuid | 0.4.45 / 1.29.0 |
| anyhow / fs2 | 1.0.104 / 0.4.3 |
| tempfile / zeroize | 3.27.0 / 1.9.1 |
| image / base64ct | 0.25.10 / 1.8.3 |
| winresource | 0.1.34 |
| windows / getrandom | 0.62.2 / 0.4.3 |
| rand（測試） | 0.10.3 |
| russh | 0.65.0 |
| FIDO2 的 p256 | 0.13.2 → 0.14.0 |
| FIDO2 的 aes / hmac / sha2 | 0.9.3 / 0.13.0 / 0.11.1 |
| FIDO2 的 bindgen / thiserror | 0.73.2 / 2.0.21 |
| ctutils | 0.4.6 → 0.4.7 |
| prettyplease | 0.2.37 → 0.3.0 |

後續：2026-11-06 Dependabot 將 aes 升到 0.9.4（commit `68ca8cf`，manifest 仍為 `^0.9.3`）。

## RustSec

- RUSTSEC-2026-0387：原 `p256 0.13.2 → elliptic-curve 0.13.8 → crypto-bigint 0.5.5` 路徑已移除，只剩 `crypto-bigint 0.7.10`（要求 `>=0.7.6`）。已解決。
- RUSTSEC-2023-0071：仍命中，處理見 `../decisions/rsa-advisory.md`。
- `cargo audit --json`：漏洞由兩項降為一項，無其他警告；仍為失敗退出碼。

## 驗證

- Windows 工作區 62 項測試通過，含本機 FIDO2 的 22 項協定／模擬裝置測試，以及應用程式、SSH／APT、Passkey 容器與金鑰撤銷測試。
- `cargo fmt --all -- --check` 與所有功能的 Clippy 通過。
- Windows release 建置；以更新前的 release 建立獨立測試容器及備份，再以新版開啟並核對主機憑證。未讀取使用者正式資料。
- Linux 本機缺 Rust、C 編譯器、pkg-config、clang、libfido2 標頭，未做 Linux 原生建置；Linux 由 GitHub Actions 建置與測試。
- 未重新操作實體 USB／Hello 驗證器；先前硬體成功回報不代表此次升級已完成所有裝置回歸測試。
