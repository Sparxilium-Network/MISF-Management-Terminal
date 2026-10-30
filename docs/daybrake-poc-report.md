# DayBrake POC 回報：MISF SQLite 儲存安全

執行日期：2026-10-31

狀態：Ubuntu POC 驗證完成；D1 的旁置 DLL 執行測試需要 Windows，無法在本環境執行。

## 環境與方法

- 作業系統：Ubuntu 沙盒。
- MISF：使用 `target/debug/misf-core` JSON CLI 與 `--debug.dir=<新建暫存目錄>`；未啟動桌面 UI。Ubuntu 無法原生執行 Windows `misf.exe`。
- 可用測試引擎：Python `sqlite3`，SQLite **3.46.1**；SQLite CLI 已安裝。此 Python SQLite 版本不是 MISF bundled SQLite **3.54.1**。
- POC 指令：`python3 poc/daybrake/probe_misf_database_cases.py`、`python3 poc/daybrake/probe_storage_operations.py`、`python3 poc/daybrake/probe_file_lock_and_permissions.py`、`python3 poc/daybrake/probe_fake_v2_migration.py`、`python3 poc/daybrake/repro_wal_schema_mutation.py`。
- 已通過：`rustup run stable cargo test --locked --workspace`；FIDO2 28、UI 3、core 38、workflow 17，合計 86 tests，0 failed。
- 另以暫時新增並在測試後還原的測試函式驗證舊 LUKSbox `/hosts.json` 匯入，以及 E1 loopback 假 SSH 伺服器；兩項各 1 test 通過。來源碼已確認還原。
- A 類產品測試：`python3 poc/daybrake/probe_misf_database_cases.py`，輸入均為腳本於 `/tmp` 建立的獨立資料庫；WAL 案例由 `python3 poc/daybrake/repro_wal_schema_mutation.py` 建立後再交給 MISF CLI。
- PowerShell 不視為 Ubuntu 驗證的必要工具；Linux 建置及打包會採用等效流程。
- 測試檔案由腳本於 `/tmp/misf-daybrake-wal-*` 建立；只含 loopback 位址字串及 `POC_SECRET` 測試資料，不含真實憑證。
- 因為未能啟動 MISF，下列標為「無法判定」的項目不得視為產品通過。

## 結果摘要

| 項目 | 結論 | 證據／限制 |
|---|---|---|
| A1 惡意 INSERT trigger | 成立 | `probe_misf_database_cases.py`：CLI exit 1，回報結構不符；主檔 hash 不變、未崩潰。 |
| A2 view、額外 index、`sqlite_stat1`／`sqlite_sequence` | 成立 | 同腳本分別測 view、額外 index + `ANALYZE` 產生 `sqlite_stat1`、AUTOINCREMENT 產生 `sqlite_sequence`；全部拒絕，hash 不變。 |
| A3 欄位 SQL 僅多一個空白 | 成立 | 同腳本以 `password TEXT  NOT NULL` 建檔；CLI 拒絕，hash 不變。 |
| A4 generated column／CHECK 函式 | 成立 | 同腳本分別測 `CHECK(length(...))` 與 generated column；全部拒絕，hash 不變。 |
| A5 錯誤 application_id、user_version 0／2／99／-1 | 成立 | 同腳本五種輸入全部回傳錯誤、hash 不變、未崩潰。v0 結構不符；2／99／-1 報告需要較新版。 |
| A6 截斷、單一資料位元翻轉、頁面大小／b-tree 損壞 | **部分不成立** | 截斷、非法頁面大小與 b-tree freeblock 指標損壞均拒絕且不崩潰。單一有效資料頁位元翻轉把測試密碼 `POC_SECRET` 改成 `QOC_SECRET`；MISF 仍成功列出主機，hash 不變（它未再寫檔）。SQLite 沒有頁面內容 MAC；這是明文模式完整性限制，不是結構檢查繞過。 |
| A7 WAL 搭配惡意 sidecar | **不成立** | `repro_wal_schema_mutation.py` 直接啟動 MISF CLI：exit 1、後續回報結構不符，但開啟前 hash `880e84cde10f0443ba0c7b30953ff5901d08d07c23d02286df10276e929a7392` 變成 `ab69b00ff80e779ca46668b19329be04022223efb1b453491361b9ecbdcbaec0`，WAL 消失，`poc_steal` trigger 已進入 `hosts.db`。輸入檔由 Python SQLite 3.46.1 生成，MISF CLI 使用專案 bundled SQLite。 |
| B1 加密期間監看明文檔案 | 成立 | `probe_storage_operations.py`：移除 `hosts.db` 後做新增、刪除、鎖定、解鎖與 2 MiB 備註寫入；未見 DB/journal/WAL/shm 或明文暫存檔重現。另植入一個 POC journal，確認明文刪除流程一併清除。 |
| B2 在容器密文搜尋測試密碼 | 成立 | 同腳本掃描 `hosts.lbx`，未找到兩組測試主機密碼或解鎖密碼。 |
| B3 明文刪除後 `secure_delete` | 成立 | 同腳本用 MISF CLI 新增後刪除測試主機；`hosts.db`（28,672 bytes）中不再找到 `POC_DELETED_SECRET`。 |
| B4 寫入中斷、journal 復原及清理 | 成立 | 同腳本寫入 6 MiB 測試備註，觀察到 `hosts.db-journal` 後以 SIGKILL 終止 writer。journal 當時仍在；下次 MISF `list` 成功恢復，只剩原有 `stable` 主機，`quick_check=ok`。journal 刪除流程另由 B1 的植入 journal 驗證。 |
| C1 明文／容器內 JSON 匯入 | 成立 | `probe_storage_operations.py` 證明明文 JSON 資料匯入、密碼一致、來源 JSON hash 不變。另以暫時加入後即還原的 Rust 測試建立僅含 `/hosts.json` 的 LUKSbox 容器；解鎖讀取後首次儲存建立 `/hosts.db` 並移除 `/hosts.json`，兩筆主機重讀成功。 |
| C2 v1→v2 假遷移、失敗回滾、降級拒絕 | 成立 | `probe_fake_v2_migration.py` 暫時加入 fake v2 並在 finally 還原來源。成功升級後有 v1 備份且 hash 與原檔相同；第二條 DDL 失敗後版本仍為 v1、欄位不存在、主檔 hash 不變；原 v1 CLI 拒絕 v2 且 hash 不變。備份 mode 為 0600。 |
| D1 SQLite DLL 載入／旁置 DLL | 部分成立 | `objdump -p dist/MISF/misf.exe` 的 PE import 清單沒有 `sqlite3.dll`，`strings` 找到 `3.54.1`。此為既有 Windows PE 的靜態檢查；本次未在 Windows 執行旁置 DLL 測試，亦未以本次 Linux 建置更新該 PE。 |
| D2 多程序互斥及 5 秒 SQLite 鎖逾時 | 成立 | `probe_file_lock_and_permissions.py` 同時寫入兩個 MISF JSON CLI，兩者 exit 0，最終保留 seed、writer-a、writer-b 三筆。SQLite CLI 持有 `BEGIN EXCLUSIVE` 時，MISF 約 5.01 秒回報 `database is locked`。 |
| D3 Unix DB／備份檔權限 0600 | 成立 | 同腳本新建 `hosts.db` mode 為 0600；C2 的實際升級備份 `hosts.v1.bak.db` mode 亦為 0600。 |
| E1 同帳號主機資料竄改劫持 | 成立（明文）；成立（加密阻擋） | 暫時加入的 loopback workflow POC 把資料庫主機位址、埠與指紋改成測試 SSH server；MISF debug CLI 僅連 loopback，假伺服器只接受 POC 密碼 `ssh-secret`，認證成功並執行偵測命令。加密重測未在密文中找到主機密碼/位址/帳號；翻轉容器位元後解鎖失敗。測試程式碼已還原，僅使用 loopback 和虛構資料。 |

## A7 重現紀錄

指令：

```bash
python3 poc/daybrake/repro_wal_schema_mutation.py
```

一次執行的觀察輸出：

```text
sqlite_version=3.46.1
before_main_sha256=630694c400bce9f66c6bbeb80c8f3941d3a8f7a1c465784f35a302a3ecc6257a
before_wal_bytes=4152
journal_mode_result=delete
after_main_sha256=fdce98fb0c2c15add6bf7c2fa18611be439ce50fcae971c6ca7f9353885678f4
main_file_changed=True
wal_still_exists=False
recovered_extra_trigger_count=1
stored_password_still_readable=True
```

這次產品層 CLI 實測確認 SQLite 在設定 `journal_mode=DELETE` 時先套用 WAL 交易，再於後續 schema 比對拒絕資料庫；拒絕前來源檔已改寫。trigger 定義已寫入主資料庫，但此讀取請求沒有插入主機，因此未觀察到 trigger 執行。重現腳本：[repro_wal_schema_mutation.py](../poc/daybrake/repro_wal_schema_mutation.py)。

## 修正建議（初步）

1. **高：** 在信任資料庫及 sidecar 前，避免以可寫連線切換 journal mode。對含 WAL／journal sidecar 的外來檔案先採拒絕策略，或使用能保證不回寫來源的驗證流程；回歸測試須斷言拒絕後來源 hash 不變、sidecar 不被消耗。
2. **中：** 修正「任意位元翻轉必定拒絕」這項測試期待與相關說明。普通 SQLite 頁面沒有認證碼，合法資料頁內的單一位元修改可被當成合法新值；明文模式此限制應明確歸入同帳號可竄改風險。若產品要求偵測此類修改，需要額外完整性金鑰或使用加密主機庫。
3. **中：** 把 A1–A7、B、C、D 的雜湊／sidecar／復原檢查固定成隔離 POC；Windows 旁置 DLL 實際載入測試仍需 Windows runner。

## MISF 後續處理（2026-10-31）

本節由 MISF 補充，以上為 DayBrake 原始回報。`poc/daybrake/` 腳本保留在 DayBrake 的測試環境，未加入本儲存庫。

1. A7：已修正。開檔前若有 `hosts.db-wal`／`-shm`，或檔頭不是 rollback journal 模式，就不交給 SQLite 並拒絕；加密映像同樣檢查檔頭。回歸測試 `wal_databases_are_refused_before_sqlite_replays_or_rewrites_them` 以相同手法（WAL 內新增 trigger）在 Windows bundled SQLite 3.54.1 重現：移除修正時原檔被改寫、測試失敗；修正後主檔與 `-wal` 位元組不變。
2. A6：任務書與[安全評估](sqlite-security.md)已改為明確說明明文模式無法偵測合法值的位元翻轉。
3. 偽造的 `hosts.db-journal` 仍會為了中斷復原而先回滾主檔；結果照樣經過所有檢查，列為已知限制 T16。
4. D1 的 Windows 旁置 DLL 實際載入測試尚未執行。
