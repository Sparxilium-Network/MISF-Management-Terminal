# 依賴更新與安全公告

檢查日期：2026-10-30（Asia/Taipei）。來源為 crates.io 官方 API、Cargo
registry 與 RustSec。應用程式及本機 FIDO2 副本的直接 registry 依賴已對齊當日
最新穩定版，共核對 27 種直接 registry 套件（含同日加入的圖示建置套件）；`Cargo.lock` 保存可重現的實際版本。未為了版本數字而強制覆蓋上游
不相容的間接依賴，因此不能宣稱依賴樹每個節點都是全域最新版。

## 更新內容

多數應用程式依賴原本已解析至最新版；這次更新 manifest 的版本下限、重新解析
所有相容的間接依賴，並將應用程式的 `rand_core 0.6` OS 亂數呼叫改為
`getrandom 0.4.3`。UI／SSH／容器功能沒有因版本更新而停用。

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
| winresource（Windows 圖示建置） | 0.1.34 |
| windows / getrandom | 0.62.2 / 0.4.3 |
| rand（測試） | 0.10.3 |
| russh | 0.65.0 |
| FIDO2 的 p256 | 0.13.2 → 0.14.0 |
| FIDO2 的 aes / hmac / sha2 | 0.9.3 / 0.13.0 / 0.11.1 |
| FIDO2 的 bindgen / thiserror | 0.73.2 / 2.0.21 |
| ctutils | 0.4.6 → 0.4.7 |
| prettyplease | 0.2.37 → 0.3.0 |

LUKSbox 最新發布仍為 v0.6.1，主分支也停在既有固定 commit。為了升級其 FIDO2
使用的 P-256，專案加入有來源與修改紀錄的
[`third_party/LUKSbox/luksbox-fido2`](../third_party/LUKSbox/luksbox-fido2/)。
這是本機相容性修改，不是宣稱上游發布了修正版。容器 core／format／VFS／PQ
維持原固定來源，沒有變更加密容器或金鑰槽格式。詳見
[修改紀錄](../third_party/LUKSbox/PATCHES.md)。

## RustSec 結果

- [RUSTSEC-2026-0387](https://rustsec.org/advisories/RUSTSEC-2026-0387.html)：
  原有 `p256 0.13.2 → elliptic-curve 0.13.8 → crypto-bigint 0.5.5`
  路徑已移除。現在只有 `crypto-bigint 0.7.10`，符合公告要求的 `>=0.7.6`。
- [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071)：
  仍命中 `rsa 0.10.0-rc.18`，公告目前沒有任何已修補版本。
  `russh 0.65.0` 精確要求 `=0.10.0-rc.18`，因此不能直接更新至已發布的
  rc.19；上游 `rsa` 的較新版也不能據此當作 Marvin Attack 的修補。
  現有 RSA SSH 功能保留，未加入 audit ignore 或標示漏洞已解決。
  掃描命中不等於已證明 MISF 的 SSH 用戶端簽章流程可被該攻擊利用。

`cargo audit --json` 的結果由原本兩項漏洞降為一項，且沒有其他警告。
由於 RSA 公告仍在，`cargo audit` **仍會以失敗退出碼結束**，不可把整體掃描
宣稱為通過。

其他受上游限制的例子：`crypto-common 0.1.7` 精確要求
`generic-array =0.14.7`；雖然已有 0.14.9，也不能僅靠 lockfile 更新取代。
LUKSbox 容器仍需要它原本的 AES／Argon2 等 API 世代，上游容器原始碼維持原樣，
沒有被整批改寫成另一個密碼實作。

## 驗證

- Windows 工作區 62 項測試通過，包含本機 FIDO2 的 22 項原有協定／模擬裝置
  測試，以及應用程式、SSH／APT、Passkey 容器與金鑰撤銷測試。
- `cargo fmt --all -- --check` 與所有功能的 Clippy 驗證。
- Windows release 建置；以更新前的 release 建立獨立測試容器及備份，再以
  新版開啟並核對主機憑證。測試不讀取使用者正式資料。
- Linux 本機環境缺少 Rust、C 編譯器、pkg-config、clang 及 libfido2 標頭，
  尚未執行 Linux 原生建置；GitHub Actions 仍有 Linux 建置與測試工作。
- 尚未重新操作實體 USB／Hello 驗證器。先前硬體成功回報不代表本次升級已
  完成所有裝置的回歸測試。

維護檢查可執行 `cargo update --verbose`、`cargo tree --locked -i crypto-bigint`
及 `cargo audit`。更新相容性副本時，應保留協定測試向量與舊容器解鎖測試，
避免只以安全掃描結果判斷相容性。
