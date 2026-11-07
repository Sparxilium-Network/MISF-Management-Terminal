# SQLite 儲存安全（DayBrake）

2026-10-31。完整內容在 `.daybrake/`（DayBrake 專屬目錄，不搬動）：

- `.daybrake/20261031-sqlite-storage-security/sqlite-security.md`：評估與主張。
- `.daybrake/20261031-sqlite-storage-security/daybrake-poc-brief.md`：任務書。
- `.daybrake/20261031-sqlite-storage-security/daybrake-poc-report.md`：Ubuntu POC 結果。
- `.daybrake/20261031-sqlite-storage-security/after-patch-report.md`：修補後重測（`470927f`）。
- `.daybrake/Know.md`：已知安全問題（A6）。

結論摘要：

- A7 惡意 WAL：已修正，有 `-wal`／`-shm` 時拒絕開啟且不改動檔案。
- A6 合法資料頁位元翻轉：SQLite `quick_check` 不偵測；鎖定中的 `hosts.lbx` 被 LUKSbox 認證拒絕。報告內「明文模式」的描述是當時狀態；2026-11-07 起已無明文模式，只剩建立容器時讀取舊版 `hosts.db` 這一次。
- 報告為歷史紀錄，不隨後續程式變更改寫。
