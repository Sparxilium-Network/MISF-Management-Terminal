# MISF Management Terminal

[GitHub 儲存庫](https://github.com/Sparxilium-Network/MISF-Management-Terminal)

> [!WARNING]
> **本專案仍處於開發階段，請勿用於正式環境。**
> 相關安全保護機制尚未完善；密碼、SSH 私鑰及其他敏感連線資料目前皆以明文儲存。

全 Rust 原生桌面客戶端，以 eframe／egui 繪製畫面，內建 Rust SSH 核心。畫面與核心編譯成同一個執行檔，不需要 Java、瀏覽器或管理伺服器。

目前沒有 PIN 或本機資料加密。帳密、匯入的私鑰與最近一次 APT 結果，以明文 JSON 保存在使用者目錄 `.misf-management/hosts.json`；UI 會顯示完整路徑。用 `MISF_DATA_DIR` 可指定資料資料夾。

首次啟動會自動複製舊版 `.msif-management/hosts.json` 至新資料目錄，保留原檔，且不覆寫已存在的新資料。

## 啟動與打包

開發需要 Rust 1.95 以上與該作業系統的原生建置工具（Windows 使用 MSVC）。

```powershell
.\run.ps1
```

建置與產生可攜式 Windows 執行檔：

```powershell
.\build.ps1
.\package.ps1
```

打包後直接執行 `dist/MISF/misf.exe`。不需要安裝 Rust 或 Java。

Linux／macOS 可使用 `cargo run --locked -p misf`，須在對應平台建置；Linux 建置需要 OpenGL、X11／Wayland、fontconfig 與檔案對話框相關開發套件。此版本已在 Windows 建置與驗證。

## 功能

- 保存多台 Linux／BSD 主機，支援新增、編輯、刪除與搜尋。
- SSH 帳密登入，或匯入 OpenSSH／PEM 私鑰；加密私鑰可填密語。
- 首次連線先核對並保存 SHA-256 主機指紋，確認前不提交帳密；指紋變更立即阻擋認證。
- 使用 russh 自帶 SSH，不呼叫系統 ssh，也不讀取電腦的 SSH config、known_hosts、SSH agent 或系統私鑰。
- 一鍵 APT 檢查：root 執行 apt-get update，一般使用者使用 sudo；更新索引成功後列出可更新套件、目前版本、候選版本與架構。只檢查、不安裝套件。
- 可開關 PTY（預設開啟，類似 ssh -t）。sudo 密碼以 SSH channel 標準輸入傳送，不放在命令列。
- 失敗不顯示為零筆更新；保留最近一次檢查結果與遮蔽憑證後的輸出。

BSD 支援 SSH 連線資料與連線測試，目前不提供 BSD 套件檢查。

## 使用

新增主機後，選擇密碼或匯入私鑰。一般帳號可填 sudo 密碼；留空時沿用 SSH 密碼，Key 登入則需另填或設定免密 sudo。先核對主機指紋，再按「檢查 APT 更新」。APT 索引更新上限 5 分鐘，逾時會報錯；關閉程式會關閉本機連線，不能保證中止遠端已開始的 apt-get。

## 測試

```powershell
cargo test --locked --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
```

Rust 整合測試在 loopback 啟動測試 SSH 伺服器，驗證密碼／私鑰、指紋、PTY、sudo 與 APT 檢查，不接觸真實遠端主機。

## 結構

- `app/`：eframe／egui 原生 UI；背景執行緒執行網路及磁碟作業。
- `core/`：主機資料、SSH 及 APT 核心；亦提供 stdin/stdout JSON CLI。
- `docs/architecture.md`：目前實作架構。

開發用畫面擷取可使用 `cargo build -p misf --features capture`，以 `MISF_SCREENSHOT_TO` 指定 PNG 路徑；`MISF_OPEN_EDITOR=1` 可擷取新增主機對話框。正式 release 預設不啟用此功能。
