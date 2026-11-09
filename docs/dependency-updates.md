# 依賴更新與安全公告

> 最後修改：2026-11-10 ｜ 基於 commit：`cc7ba2e`

依賴的實際版本以已提交的 `Cargo.lock` 為準；建置與發布一律使用 `--locked`，不在建置時改變依賴。Dependabot 每日檢查 Actions 與 Cargo 依賴並提出更新 PR，通過驗證及合併後才進入發布。

## LUKSbox 與 FIDO2 相容性副本

LUKSbox 最新發布仍為 v0.6.1，主分支也停在既有固定 commit。為了升級其 FIDO2
使用的 P-256 等依賴，專案加入有來源與修改紀錄的
[`third_party/LUKSbox/luksbox-fido2`](../third_party/LUKSbox/luksbox-fido2/)。
這是本機相容性修改，不是上游發布的修正版。容器 core／format／VFS／PQ
維持原固定來源，沒有變更加密容器或金鑰槽格式。詳見
[修改紀錄](../third_party/LUKSbox/PATCHES.md)。

## 不支援 RSA

`core/Cargo.toml` 以 `default-features = false, features = ["ring"]` 引用 russh，
不啟用 `rsa` 功能，`rsa` 套件不在依賴樹中。支援的金鑰為 Ed25519 與 ECDSA
（nistp256／nistp384／nistp521）；RSA 私鑰在匯入與連線時都會被拒絕，只提供
RSA 主機金鑰的伺服器無法連線。

`rsa` 套件的 Marvin Attack 公告
[RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071) 沒有已修補版本，
russh 又精確鎖定其 pre-release 版本。除非上游發布修補且 russh 允許使用，否則不可為了相容性重新啟用。

其他受上游限制的例子：`crypto-common 0.1.7` 精確要求
`generic-array =0.14.7`，不能僅靠 lockfile 更新取代。LUKSbox 容器仍需要它原本的
AES／Argon2 等 API 世代，上游容器原始碼維持原樣，沒有改寫成另一個密碼實作。

## 維護檢查

- 可執行 `cargo update --verbose`、`cargo tree --locked -i crypto-bigint`
  及 `cargo audit`；`cargo tree --locked -i rsa` 應找不到套件。
- 更新相容性副本時，應保留協定測試向量與舊容器解鎖測試，避免只以安全掃描結果判斷相容性；並重新以實體驗證器測試（見 [Passkey 實體測試](passkey-testing.md)）。

## 授權清單

更新 `Cargo.lock` 後須執行 `./licenses.ps1`：先以 `cargo deny check licenses`
依 `deny.toml` 確認所有相依授權與 AGPL-3.0 相容，再以 cargo-about 重新產生
`licenses/THIRD_PARTY_LICENSES.html`，並提交產生的檔案。清單記錄所依據的
`Cargo.lock` 雜湊；不一致時 `package.ps1`（含 CI）拒絕打包。雙授權套件依
`about.toml` 的順序優先採用 MIT（附各套件版權行、無 NOTICE 義務）；目前只採
Apache-2.0 的套件皆無 NOTICE 檔，LUKSbox 的 NOTICE 另附於 `licenses/LUKSbox/`。
Slint 升級時，須確認其授權選項仍含 GPL-3.0-only，並確認 Windows／Linux 版沒有另外內嵌 Slint 自帶的字型（`i-slint-common` 的 Inter 字型目前只在 wasm／QNX 內嵌）；介面字型是專案自帶的 Noto Sans TC。Skia 由 `skia-bindings` 下載預先編譯檔，升級時需更新 `licenses/Skia/LICENSE`（取自建置輸出的 `LICENSE_SKIA`）；並依新版 Skia `DEPS` 釘選的版本，更新 `licenses/Skia/third_party/` 內各元件的授權全文與 `THIRD_PARTY_NOTICES.md` 的表格。實際編入的元件可用 `skia.lib` 的符號確認（例如 `png_create_read_struct`、`jpeg_read_header`、`XML_ParserCreate`、`wuffs_gif`、`D3D12MA`）。
