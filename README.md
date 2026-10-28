# MISF Management Terminal

[GitHub 儲存庫](https://github.com/Sparxilium-Network/MISF-Management-Terminal)

> [!WARNING]
> **本專案仍處於開發階段，請勿用於正式環境。**
> 相關安全保護機制尚未完善。未啟用 LUKSbox 時，密碼、SSH 私鑰及其他敏感資料以明文儲存；啟用後仍會保留原明文副本，請自行確認與處理。LUKSbox 整合為實驗性功能，不能視為正式安全保證。

全 Rust 原生桌面客戶端，以 eframe／egui 繪製畫面，內建 Rust SSH 核心。畫面與核心編譯成同一個執行檔，不需要 Java、瀏覽器或管理伺服器。

預設將主機與憑證存於使用者目錄 `.misf-management/hosts.json`。可選擇啟用 **LUKSbox 加密主機庫**，之後日常資料改存同目錄的 `hosts.lbx`，每次啟動先解鎖。用 `MISF_DATA_DIR` 可指定資料資料夾。

首次啟動會自動複製舊版 `.msif-management/hosts.json` 至新資料目錄，保留原檔，且不覆寫已存在的新資料。

## 啟動與打包

開發需要 Rust 1.95 以上、Git 與該作業系統的原生建置工具（Windows 使用 MSVC）。

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

## 自動建置與 Pre-release

[GitHub Actions](https://github.com/Sparxilium-Network/MISF-Management-Terminal/actions/workflows/build-prerelease.yml) 在 `main` 更新、推送 `v*` 標籤或手動執行時，自動測試並建置 **Linux x86_64** 與 **Windows x86_64**。兩個平台的格式、Clippy、測試與 release 建置全部通過後，才上傳套件並公開 **Pre-release**，固定不標記為 Latest。PR 只驗證與產生 Actions artifacts，不發布。

- Windows：`MISF-Management-Terminal-windows-x86_64.zip`，解壓後執行 `misf.exe`。
- Linux：`MISF-Management-Terminal-linux-x86_64.tar.gz`，解壓後執行 `./misf`。這是原生動態連結套件，需要圖形桌面及相容的 glibc、OpenGL、X11／Wayland、fontconfig 等系統函式庫；不是 AppImage，也不保證支援較舊發行版。
- 每個套件內附 README、架構／加密評估文件及第三方授權；另附 SHA-256 校驗檔。

`main`／手動執行（選 `main`）使用 `dev-<run_number>.<run_attempt>-<短 commit>` 作為版本標籤；推送 `v*` 標籤時沿用該標籤。即使標籤為 `v1.0.0`，本流程仍只發布 Pre-release。已公開的同名版本不會被重跑覆寫；上傳未完成的草稿可重跑補齊。GitHub 的 Pre-release 仍顯示在 Releases 頁面，只是標記為預發行版，不是正式發行。

建置環境使用 GitHub 的 `ubuntu-latest`／`windows-latest` 和每次安裝的最新 **Rust stable**（不是 nightly）。Actions 使用目前最新穩定版的固定 commit；Dependabot 每日檢查 Actions 與 Cargo 依賴並提出更新 PR，通過驗證及合併後才進入發布。套件使用已提交的 `Cargo.lock` 與 `--locked`，不在發布時偷偷改變依賴；LUKSbox 仍固定在已驗證的版本。Linux 的 glibc 相容性下限會隨 `ubuntu-latest` 映像更新。

本機也可用 PowerShell 7 在對應平台執行 `./package.ps1 -Archive`，產生相同壓縮檔及校驗檔。GitHub Actions 僅發布套件，不會提交原始碼；本機操作仍遵循 AGENTS.md 的禁止 push 規則。此工作流程需先由維護者推送至 GitHub 才會啟用，首次遠端執行結果請於 Actions 查看。

## 功能

畫面頂端可選 **繁體中文（台灣）／English**，以及 **跟隨系統／淺色／深色**；切換立即生效，不需要重啟。偏好另存於資料目錄的 `ui-settings.json`，下次啟動自動沿用，主機庫鎖定時也能調整。此檔僅含語言與外觀設定，不保存憑證。

介面文字放在 [`app/locales/zh-tw.json`](app/locales/zh-tw.json) 與 [`app/locales/en-us.json`](app/locales/en-us.json)，以固定 key 與 `{name}` 等命名參數維護。語系檔編譯進執行檔，發行套件不需另外安裝語言檔；修改 JSON 後需重新編譯。新增語言時，在 `app/src/i18n.rs` 登記並提供相同 key 與參數。測試會檢查 key 與參數是否一致；缺少翻譯時回退繁體中文。遠端命令原始輸出不翻譯，核心／系統的詳細錯誤亦保留原文。

- 保存多台 Linux／BSD 主機，支援新增、編輯、刪除與搜尋。
- SSH 帳密登入，或匯入 OpenSSH／PEM 私鑰；加密私鑰可填密語。
- 首次連線先核對並保存 SHA-256 主機指紋，確認前不提交帳密；指紋變更立即阻擋認證。
- 使用 russh 自帶 SSH，不呼叫系統 ssh，也不讀取電腦的 SSH config、known_hosts、SSH agent 或系統私鑰。
- 一鍵 APT 檢查：root 執行 apt-get update，一般使用者使用 sudo；更新索引成功後列出可更新套件、目前版本、候選版本與架構。只檢查、不安裝套件。
- 可開關 PTY（預設開啟，類似 ssh -t）。sudo 密碼以 SSH channel 標準輸入傳送，不放在命令列。
- 失敗不顯示為零筆更新；保留最近一次檢查結果與遮蔽憑證後的輸出。

BSD 支援 SSH 連線資料與連線測試，目前不提供 BSD 套件檢查。

## 可選的 LUKSbox 加密儲存

1. 按資料區底部的「啟用 LUKSbox 加密…」，設定至少 12 字元的解鎖密碼。
2. 程式建立容器、將資料直接從記憶體寫入，再關閉並重新解鎖驗證。原 `hosts.json` 保留；介面持續提示它仍是明文副本。
3. 啟用後，新讀寫只使用 `hosts.lbx`；`hosts.lbx.enabled` 記錄已選擇加密模式。鎖定、錯誤密碼或容器遺失時不會退回讀取明文 JSON。按「鎖定」後，必須再次解鎖才能操作主機。
4. 透過「管理解鎖金鑰」新增備援密碼。最多 8 組密碼可分別解鎖同一容器，不會因增加密碼而重新加密所有主機資料。
5. 按「匯出加密備份…」建立新的 `.lbx` 備份檔。請保留可取回的備援密碼，並實際驗證備份可解鎖；遺失所有密碼將無法復原。

使用 LUKSbox v0.6.1（固定 commit `69448f708f1328d672deb595e359579d630cb0b0`）的 core／format／VFS 函式庫，不掛載磁碟、不需要 WinFsp、FIDO2 或 TPM 驅動。加密為 AES-256-GCM-SIV；密碼採 Argon2id（256 MiB、3 次、4 lanes）。這不是 LUKS2 磁碟格式。

此版僅支援新增密碼 keyslot，尚未提供撤銷、改密碼、主金鑰輪替、FIDO2／TPM／passkey 或自動閒置鎖定。備份或舊副本不會隨目前容器的變更自動失效。加密只保護未解鎖的檔案，無法保護已遭入侵的電腦。

原 `hosts.json`、舊 `.msif-management` 目錄、外部備份與磁碟殘留不會自動清除；請在驗證容器和備份後自行處理。啟用後請勿再用不支援 LUKSbox 的舊版操作同一資料目錄，以免重新更新明文副本。看到 `.lbx.header-bak`／`.lbx.meta-bak` 時，這些是上游的加密復原鏡像，請與容器妥善保存。主機資料序列化內容上限 64 MiB。

可用 [LUKSbox 原生工具](https://github.com/PentHertz/LUKSbox) 直接開啟備份；其容器內保存 `/hosts.json`。若要讓 MISF 使用備份，請先關閉程式，以備份建立獨立資料目錄並命名為 `hosts.lbx`，再透過 `MISF_DATA_DIR` 指向該目錄；避免覆寫目前主資料。詳細評估見 [LUKSbox 整合評估](docs/luksbox-evaluation.md)。

## 使用

新增主機後，選擇密碼或匯入私鑰。一般帳號可填 sudo 密碼；留空時沿用 SSH 密碼，Key 登入則需另填或設定免密 sudo。先核對主機指紋，再按「檢查 APT 更新」。APT 索引更新上限 5 分鐘，逾時會報錯；關閉程式會關閉本機連線，不能保證中止遠端已開始的 apt-get。

## 測試

```powershell
cargo test --locked --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
```

Rust 整合測試在 loopback 啟動測試 SSH 伺服器，驗證密碼／私鑰、指紋、PTY、sudo 與 APT 檢查，不接觸真實遠端主機。

LUKSbox 測試使用暫存資料，涵蓋明文保留、鎖定後拒絕操作、錯誤密碼、多 keyslot、重新啟動、加密備份、縮短資料、密文損壞及並行開啟阻擋。

## 結構

- `app/`：eframe／egui 原生 UI；背景執行緒執行網路及磁碟作業。
- `core/`：主機資料、SSH 及 APT 核心；亦提供 stdin/stdout JSON CLI。
- CLI 的加密工作階段只存在同一程序內；可透過 stdin 傳入 JSON 請求陣列，依序執行解鎖、操作與鎖定。不要把解鎖密碼放進命令列參數。
- `docs/architecture.md`：目前實作架構。

開發用畫面擷取可使用 `cargo build -p misf --features capture`，以 `MISF_SCREENSHOT_TO` 指定 PNG 路徑；`MISF_OPEN_EDITOR=1` 可擷取新增主機對話框。正式 release 預設不啟用此功能。
