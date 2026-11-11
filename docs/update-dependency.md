# 更新依賴的步驟

> 最後修改：2026-11-12 ｜ 基於 commit：`b64f596`

本文件是實際操作清單：收到 Dependabot PR、想手動升級某個套件，或 CI 因安全公告失敗時，照這裡的步驟做。各依賴的背景、上游限制與已知例外見 [依賴更新與安全公告](dependency-updates.md)。

## 需要的工具

```powershell
cargo install --locked cargo-deny
cargo install --locked cargo-about --features cli
```

`licenses.ps1` 會用到這兩個工具。

## Dependabot 會做什麼、不會做什麼

- `.github/dependabot.yml` 每天檢查 Cargo 與 GitHub Actions。相容的 Cargo 更新合併成一個 `rust-compatible` 群組 PR；超出目前版本需求、必須修改 `Cargo.toml` 的升級會單獨開 PR（例如 rfd 0.17 → 0.18）。
- 沒有設定 `target-branch`，所以 PR 只會以預設分支 `main` 為目標，`legacy-egui` 不會收到 PR。
- Dependabot **不會**執行 `./licenses.ps1`。只要 `Cargo.lock` 有改動，PR 的 CI 一定會在授權清單檢查失敗（`licenses/THIRD_PARTY_LICENSES.html is out of date with Cargo.lock`）。這不代表更新有問題，但也表示 PR 不能直接合併。

## 套用 Dependabot PR

1. **看失敗原因**：`gh pr checks <編號>`，再用 `gh run view --job <job-id> --log-failed` 看實際錯誤。若只有授權清單過期，繼續下一步；若是編譯或測試失敗，先處理那個問題。
2. **看改了什麼**：`gh pr diff <編號>`。記下 `Cargo.toml` 與 `Cargo.lock` 的每一處改動，作為之後比對的標準。
3. **看上游變更**：0.x 次版號或大版號升級要讀發行說明（例如 `gh api repos/<owner>/<repo>/releases`），確認：
   - 本專案用到的 API 是否有破壞性變更（用 `grep -rn "<crate>::" app/src core/src` 找使用處）。
   - MSRV 是否高於目前的 stable 工具鏈。CI 使用 `rustup` 的 stable。
   - 是否新增預設功能、平台後端或新的間接依賴，以及它們的授權。
4. **在本機 `main` 套用**：
   - 若 PR 改了 `Cargo.toml`，先手動改成相同版本。
   - 執行 `cargo update -p <套件> --precise <版本>`；群組 PR 每個套件各跑一次。
5. **比對鎖檔，最容易出錯的一步**：`git diff Cargo.lock`。`cargo update -p` 常會連帶重新解析無關套件，實際發生過：
   - windows-sys／getrandom 被換成較舊版本。
   - bindgen 的 `syn 3` 被換成 `syn 2`，導致 bindgen 編譯失敗。
   
   差異必須與步驟 2 記下的完全相同；多出來的改動要手動改回，或在 `Cargo.lock` 中直接照 PR 修改。改完執行 `cargo metadata --locked --format-version 1 > $null`，確認鎖檔與 `Cargo.toml` 一致。
6. **重新產生授權清單**：`./licenses.ps1`。它會先跑 `cargo deny check licenses`，再用 cargo-about 產生 `licenses/THIRD_PARTY_LICENSES.html`，並把 `Cargo.lock` 的雜湊寫進 `licenses/THIRD_PARTY_LICENSES.lock-sha256`。之後若又改了 `Cargo.lock`（例如步驟 5 的修正），要再執行一次。
7. **完整檢查**：
   ```powershell
   cargo deny --locked check
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets --all-features -- -D warnings
   cargo test --workspace
   ```
   `cargo deny` 最後一行應為 `advisories ok, bans ok, licenses ok, sources ok`。
8. **記錄並提交**：
   - 在 `.cowork/verification/workspace-tests.md` 最上方加一筆：套用了哪些 PR、鎖檔差異、手動改回了什麼、檢查結果與未驗證項目。
   - 只用明確路徑 `git add`，通常是 `Cargo.lock`、有改到的 `Cargo.toml`、`licenses/THIRD_PARTY_LICENSES.html`、`licenses/THIRD_PARTY_LICENSES.lock-sha256` 與驗證紀錄。
   - commit 訊息依專案慣例：英文標題、繁體中文說明。
9. **打包並實際試用**：`.\package.ps1`，比對 `dist\MISF\misf.exe` 與 `target\release\misf.exe` 的雜湊與修改時間。測試只涵蓋邏輯；跟介面或系統互動有關的依賴要在 app 裡實際操作，例如：
   - rfd：匯入／匯出與 LOG 匯出的檔案對話框。
   - russh：連線真實主機。
   - Slint／winit：視窗、字型與輸入。
10. **推送後關閉 PR**：本機 commit 推送到 `main` 後，Dependabot 偵測到依賴已更新，通常會自動關閉 PR；沒有關閉就手動關閉，並註明由哪個 commit 套用。推送前要先取得儲存庫擁有者同意。

## 手動升級套件

步驟與上面相同，只是沒有 PR 可以比對：

- 在 `Cargo.toml` 改版本需求後，執行 `cargo update -p <套件> --precise <版本>`。
- 用 `git diff Cargo.lock` 確認只改了該套件與它新需要的依賴；其他套件被降版或換版本就改回去。
- 加入新的依賴屬於重大決定，要先與擁有者討論，並確認授權在 `deny.toml` 的允許清單內。

## 特定套件的額外工作

| 套件 | 額外要做的事 |
| --- | --- |
| Slint | 確認授權仍含 GPL-3.0-only；確認 slint-ui/slint#13655 是否已修正，修正後移除 `widgets.slint` 的 `min-height: self.preferred-height`，且 `pages_respond_to_clicks` 仍須通過；用截圖建置檢查各頁版面。 |
| skia-bindings（Skia） | 更新 `licenses/Skia/LICENSE` 與 `licenses/Skia/third_party/`，並依新版 Skia `DEPS` 更新 `THIRD_PARTY_NOTICES.md`（詳見 [授權清單](dependency-updates.md#授權清單)）。 |
| russh | 不可啟用 `rsa` 功能；`cargo tree --locked -i rsa` 應找不到套件。實際連線一台主機測試。 |
| LUKSbox／FIDO2 副本 | 保留協定測試向量與舊容器解鎖測試，並用實體驗證器測試（見 [Passkey 實體測試](passkey-testing.md)）。 |
| winit／AccessKit 等 Slint 間接依賴 | 升級後檢查 `deny.toml` 中已知的停止維護例外是否仍需要：`cargo tree --locked -i <套件>` 找不到就移除該例外。 |

## CI 因安全公告失敗

CI 的 `check advisories` 使用當下的 RustSec 資料庫，所以程式沒改也可能失敗。

1. 從 CI 紀錄或本機 `cargo deny --locked check advisories` 找到公告編號與受影響的套件。
2. 用 `cargo tree --locked -i <套件>` 查出是誰引入的。
3. 有修補版本：照上面的步驟升級。
4. 沒有修補版本：評估影響範圍（哪個平台、哪項功能、是否真的會用到受影響的程式碼），**先問擁有者**，同意後才在 `deny.toml` 的 `[advisories] ignore` 加入附原因的例外，並更新 [已知的停止維護公告](dependency-updates.md#已知的停止維護公告) 表格。不要只為了讓 CI 通過就加例外。
