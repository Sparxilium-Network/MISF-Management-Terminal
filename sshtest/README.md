# sshtest：套件管理器故障情境測試

> 最後修改：2026-11-16 ｜ 基於 commit：`752ab8d`

用真的容器測試 MISF 經 SSH 做的套件檢查與升級：升級到一半失敗、斷線、不正常的輸出、套件腳本跳出畫面或等人回答。`core/tests/workflow.rs` 的模擬 SSH 伺服器只會回傳寫好的文字；這裡跑的是真的 sshd、sudo 與套件管理器。

每個測試都自己啟動一個全新的容器，結束時移除，所以可以任意順序、平行、重複執行，測試時不需要網路。

## 目錄

每個套件管理器一個資料夾，裡面有自己的映像、測試與說明；共用的框架只放在 `src/`。

```text
sshtest/
├─ Cargo.toml        每個套件管理器一個 [[test]] 目標
├─ src/lib.rs        共用框架：System、Container、Relay、Lab、Account
└─ apt/              APT（Debian／Ubuntu）
   ├─ README.md      測試套件、測試項目、發現的缺口
   ├─ docker/        映像：Dockerfile、build-packages.sh、misf-scenario
   ├─ main.rs        測試目標 `apt` 的入口
   ├─ support.rs     APT 專用的輔助（讀 dpkg 狀態、共用斷言）
   └─ upgrade.rs、failures.rs、output.rs、prompts.rs、connection.rs
```

| 套件管理器 | 資料夾 | 說明 |
| --- | --- | --- |
| APT | [`apt/`](apt/README.md) | 18 項測試；`debian:13`（預設）與 `ubuntu:26.04` 通過 |
| DNF、YUM、pkg | 尚未建立 | |

## 需要什麼

- 能執行 Linux 容器的 `docker` 指令（Windows 用 Docker Desktop，Linux 用 Docker Engine）。
- 第一次執行要建置映像：從 Docker Hub 下載基底映像，並從發行版的套件庫安裝 sshd 等工具。之後由 Docker 的快取處理，約 1 秒。

## 執行

這些測試都標了 `#[ignore]`，平常的 `cargo test --workspace` 只會編譯、不會執行：

```powershell
cargo test -p misf-sshtest -- --ignored
```

連同專案其他測試一次跑完（在儲存庫根目錄）：

```powershell
cargo test --locked --workspace -- --include-ignored
```

只跑一個套件管理器：`cargo test -p misf-sshtest --test apt -- --ignored`。只跑一類或一項：在最後加上模組或測試名稱的一部分，例如 `connection` 或 `a_cut_connection`。

| 環境變數 | 作用 |
| --- | --- |
| `MISF_SSHTEST_<管理器>_BASE` | 該套件管理器的基底映像，例如 `MISF_SSHTEST_APT_BASE=ubuntu:26.04`（預設 `debian:13`）。映像名稱是 `misf-sshtest-<管理器>:<基底>`。 |
| `MISF_SSHTEST_SLOW=1` | 另外執行需要幾分鐘的測試（目前是 APT 的「網路無聲中斷」，約 4 分鐘）；沒設定時這些測試直接通過並印出略過的說明。 |

測試程序被強制結束時（例如按 Ctrl+C），容器來不及移除。列出與清除：

```powershell
docker ps -aq --filter label=misf-sshtest=1
docker rm -f (docker ps -aq --filter label=misf-sshtest=1)
```

## 接到 CI

目前沒有接進 GitHub Actions。要接的時候，在 Linux runner（`ubuntu-latest` 已內建 Docker）裝好 Rust 與 `build-prerelease.yml` 的「Install Linux build dependencies」之後加一步；每個套件管理器可以各自一個工作：

```yaml
- name: APT fault tests
  run: cargo test --locked -p misf-sshtest --test apt -- --ignored
```

Windows runner 不能執行 Linux 容器，所以只能放在 Linux 工作。這些測試會從 Docker Hub 與發行版套件庫下載，外部服務故障時會失敗；建議先用獨立、手動觸發的 workflow，不要擋住發布。

## 共用框架（`src/lib.rs`）

```text
測試 ──► misf-core（dispatch_with）──SSH──► Relay（127.0.0.1）──► 容器的 sshd ──► sudo／套件管理器
  └────────── docker exec：佈置情境、觀察主機狀態 ──────────────────────────────►
```

- `System`：一個套件管理器的測試主機（MISF 的管理器代碼，同時是資料夾名稱；以及預設基底映像）。目前只有 `APT`。
- `Container`：啟動容器、`sh`／`try_sh`／`spawn`（`docker exec`）、`wait_for`。
- `Relay`：MISF 與容器之間的 TCP 中繼。`cut` 立刻斷線並拒絕新連線，`freeze` 不關閉但不再傳資料，`restore` 恢復。
- `Lab`：像 app 一樣把主機存進主機庫、信任指紋、測試連線，再用 `dispatch_with` 執行 `check`／`upgrade`／`dist_upgrade`，並收集即時終端機輸出（`screen`）。`run` 最多等 5 分鐘（`PATIENCE`），超過就讓測試失敗，不會等到 MISF 自己的 30 分鐘上限；`run_for` 在指定秒數後放棄並斷開連線，效果等同 app 的「中斷」。
- `Account`：每個映像都要建立的三個帳號（密碼寫在 Dockerfile，只是測試值；SSH 埠只發布在 127.0.0.1）。

| `Account` | 帳號 | 說明 |
| --- | --- | --- |
| `Root` | `root` | 直接執行套件管理器 |
| `Admin` | `misfadmin` | 經 sudo，要輸入密碼 |
| `Deploy` | `misfdeploy` | 經 sudo，`NOPASSWD` |

## 新增一個套件管理器

以 YUM 為例：

1. 建立 `sshtest/yum/docker/`。映像必須：在 22 埠執行 sshd（Ed25519 或 ECDSA 主機金鑰、允許密碼與 root 登入）、建立上表三個帳號與相同的密碼、提供 `misf-scenario <套件…>` 指令把指定套件的新版發布到本機套件庫，並接受 `BASE` 建置參數。
2. 在 `src/lib.rs` 加一個 `pub const YUM: System`（`manager: "yum"`，與資料夾同名）。
3. 在 `Cargo.toml` 加 `[[test]] name = "yum"`、`path = "yum/main.rs"`。
4. 在 `yum/` 寫測試與 `README.md`；只有該管理器用得到的輔助（例如讀 rpm 狀態）放在 `yum/support.rs`，不要放進 `src/lib.rs`。
5. 在上面的表格與 `docs/PROJECT-DEV-ARCH.md` 補上。
