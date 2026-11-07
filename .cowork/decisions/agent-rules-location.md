# Agent 規則與文件放置

2026-11-07

- 使用者要求移除根目錄 `AGENTS.md`。原內容（專案命名、遠端、完成即 commit、禁止 push、保留使用者既有變更、提交後執行 `package.ps1`）移到 `docs/PROJECT-DEV.md` 的「專案慣例」。
- 依 `.claude/Common-dev-instruct.md` v2（人工維護，Agent 不得修改）：
  - `README.md` 只放使用者說明；進階用法在 `docs/user-guide.md`；開發文件入口是 `docs/PROJECT-DEV.md`。
  - `README.md` 與 `docs/` 每份文件標題下方標註最後修改日期與修改前的 `HEAD` 短雜湊。
  - 決策、驗證結果、推論放 `.cowork/`，不放 `docs/`。
  - commit message：英文標題 + 空行 + 繁體中文說明；以明確路徑 `git add`。
- 2026-11-07 已把 `docs/` 中的驗證紀錄（測試數、實測結果、日期化的檢查紀錄、「不推論」類判斷）搬到 `.cowork/verification/`。
- `docs/luksbox-evaluation.md` 改名為 `docs/luksbox-design.md`，只保留設計、限制與測試範圍。
- `.daybrake/` 是 DayBrake 安全測試的專屬目錄，維持原樣，不搬進 `.cowork/`；摘要見 `verification/storage-security-daybrake.md`。
- `assets/README.md` 與 `third_party/LUKSbox/PATCHES.md` 是素材／上游修改說明（PATCHES 為 Apache-2.0 修改聲明），不套用 docs 標頭規則，未改動。
- `.claude/` 已被 `.gitignore` 排除；其中 `AGENT.md`、`Common-dev-instruct.md` 不會被 Claude Code 自動載入。
