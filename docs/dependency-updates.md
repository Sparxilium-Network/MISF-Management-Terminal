# 依賴更新與安全公告

> 最後修改：2026-11-07 ｜ 基於 commit：`7415de8`

依賴的實際版本以已提交的 `Cargo.lock` 為準；建置與發布一律使用 `--locked`，不在建置時改變依賴。Dependabot 每日檢查 Actions 與 Cargo 依賴並提出更新 PR，通過驗證及合併後才進入發布。

## LUKSbox 與 FIDO2 相容性副本

LUKSbox 最新發布仍為 v0.6.1，主分支也停在既有固定 commit。為了升級其 FIDO2
使用的 P-256 等依賴，專案加入有來源與修改紀錄的
[`third_party/LUKSbox/luksbox-fido2`](../third_party/LUKSbox/luksbox-fido2/)。
這是本機相容性修改，不是上游發布的修正版。容器 core／format／VFS／PQ
維持原固定來源，沒有變更加密容器或金鑰槽格式。詳見
[修改紀錄](../third_party/LUKSbox/PATCHES.md)。

## 尚未修補的安全公告

- [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071)：
  命中 `rsa 0.10.0-rc.18`，公告目前沒有任何已修補版本。
  `russh 0.65.0` 精確要求 `=0.10.0-rc.18`，無法單獨更新；上游 `rsa` 的較新版
  也不能當作 Marvin Attack 的修補。RSA SSH 功能保留，未加入 audit ignore。
- 因此 `cargo audit` **會以失敗退出碼結束**，不可把整體掃描視為通過。

其他受上游限制的例子：`crypto-common 0.1.7` 精確要求
`generic-array =0.14.7`，不能僅靠 lockfile 更新取代。LUKSbox 容器仍需要它原本的
AES／Argon2 等 API 世代，上游容器原始碼維持原樣，沒有改寫成另一個密碼實作。

## 維護檢查

- 可執行 `cargo update --verbose`、`cargo tree --locked -i crypto-bigint`
  及 `cargo audit`。
- 更新相容性副本時，應保留協定測試向量與舊容器解鎖測試，避免只以安全掃描結果判斷相容性；並重新以實體驗證器測試（見 [Passkey 實體測試](passkey-testing.md)）。

## 授權清單

更新 `Cargo.lock` 後須執行 `./licenses.ps1`：先以 `cargo deny check licenses`
依 `deny.toml` 確認所有相依授權與 AGPL-3.0 相容，再以 cargo-about 重新產生
`licenses/THIRD_PARTY_LICENSES.html`，並提交產生的檔案。清單記錄所依據的
`Cargo.lock` 雜湊；不一致時 `package.ps1`（含 CI）拒絕打包。雙授權套件依
`about.toml` 的順序優先採用 MIT（附各套件版權行、無 NOTICE 義務）；目前只採
Apache-2.0 的套件皆無 NOTICE 檔，LUKSbox 的 NOTICE 另附於 `licenses/LUKSbox/`。
eframe 升級時，須對照 `licenses/egui-fonts/README.md` 確認內嵌字型及授權。
