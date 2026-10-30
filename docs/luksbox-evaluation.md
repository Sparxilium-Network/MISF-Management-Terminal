# LUKSbox 整合評估與實作

評估日期：2026-10-28 至 2026-10-29（Asia/Taipei）。使用 PentHertz/LUKSbox v0.6.1，固定 Git commit
`69448f708f1328d672deb595e359579d630cb0b0`；不追蹤浮動 main。

## 結論與範圍

Windows MSVC 的隔離原型已驗證可直接以 Rust 函式庫建立容器、用兩組密碼解鎖並讀寫檔案，不需掛載驅動。依使用者選擇，MISF 加入「可選的加密本機主機庫」，保留原生 Rust UI；現有使用者需在介面主動啟用。

採用 upstream core／format／VFS／FIDO2；容器內保存單一 SQLite 映像 `/hosts.db`（舊容器為 `/hosts.json`，下次儲存時轉換），其內容、SSH 憑證、套件結果及 VFS 中繼資料都經上游加密。未自行設計新容器格式。最多 8 個密碼／Passkey keyslot 彼此獨立解鎖，支援指定槽撤銷；不接入掛載層。Apache-2.0 的 LICENSE 與 NOTICE 一併發佈。

## 實作決定

- AES-256-GCM-SIV，密碼使用 upstream Argon2id INTERACTIVE：256 MiB、t=3、p=4；新密碼至少 12 字元，避免把短 PIN 當成能抵抗離線猜測的密碼。
- 直接使用 Container／Vfs API，解密只進記憶體，不掛載磁碟，也不把容器內容解壓到明文暫存檔。
- 上游預設 metadata 區 64 MiB，原型空間約 64 MiB；MISF 透過上游提供的 thread-local API 設為 1 MiB，因為只有一個資料檔。資料內容以加密 chunk 儲存，序列化資料上限 64 MiB。
- 每次儲存建立新的 VFS inode，寫入後 rename 並 flush；失敗的工作階段禁止繼續讀寫，需鎖定後重新解鎖確認狀態。
- 啟用前鎖定明文資料庫，建立暫存加密容器、關閉並以密碼重新解鎖比較資料，驗證後才將容器放到正式路徑。先保留原明文 JSON；目前 GUI 在啟動／解鎖後顯示完整路徑並詢問是否刪除，只有使用者明確確認且主機庫已解鎖才刪除。
- 有 `.lbx` 或持久標記 `hosts.lbx.enabled` 就選擇加密模式；鎖定、密碼錯誤、檔案損壞或容器遺失都禁止回退明文儲存。標記不含主機資料或金鑰。資料目錄改名遷移也不得在已啟用加密時重新複製舊明文。
- Windows LockFileEx 會阻止透過第二個 handle 讀取開啟中的容器。備份經現有 Container handle 複製密文，完成同步後才提交新備份檔，不覆寫既有檔案。
- 拒絕停用檔案鎖、輸出解密 metadata 或容錯資料覆寫的上游環境變數。保存密碼的請求以 stdin／記憶體佇列傳遞，密碼與序列化解密暫存緩衝區用 zeroize 清除；不宣稱所有 GUI／allocator／作業系統暫存副本都已被抹除。

## 已知限制與需要注意的問題

上游仍為 pre-1.0，說明尚未完成付費第三方稽核，並建議把容器視為可攜副本而不是唯一主副本。本次整合因此保留「開發階段、請勿正式使用」警告，加入加密備份入口，且不自動銷毀現有資料。上游的內部稽核及測試不能代替獨立安全保證。

目前沒有自動閒置鎖定、密碼原地修改、TPM 或主金鑰輪替。新增或撤銷 slot 後應重新建立加密備份；舊備份保存的是當時的 keyslot 集合。只刪 slot 不會讓攻擊者手上的舊備份失效，需另行規劃主金鑰輪替與備份生命週期。撤銷槽也不會刪除驗證器內的憑證或關閉已解鎖的工作階段。

2026-10-29 加入 Windows WebAuthn／Linux libfido2 Passkey 介面。要求相容的 PRF／hmac-secret，使用上游包裝槽、RP ID 與 salt 慣例；註冊先確認兩次 PRF 相同才寫 header，至少保留一槽。未把僅完成註冊或不支援 PRF 的驗證器當成可解鎖金鑰。Windows 使用系統使用者驗證視窗；Linux 要求當次 PIN。模擬驗證器測試涵蓋兩把不同金鑰、錯金鑰、取消、無效輸出、過長 ID、重啟後解鎖和撤銷；實體裝置與 Linux 原生建置仍須另行驗證。

原明文檔、舊版目錄及外部備份仍可能含密碼；保留副本代表整台電腦目前仍可能有明文。解鎖後資料需供 SSH 使用，加密不能防止已遭入侵的電腦或程序存取記憶體。LUKSbox 的容器復原鏡像也應妥善保存。

## 驗證

Windows 原型完成雙密碼與錯密碼驗證；MISF 自身測試覆蓋轉換保留原檔、鎖定與錯密碼拒絕、重新啟動、兩個 keyslot、加密備份可解鎖、資料縮短不留舊尾端、密文篡改失敗、並行開啟阻擋，以及原有 SSH／APT 流程。低成本 Argon2id 參數只用於測試程式碼；GUI 的正式建立及新增 slot 使用 INTERACTIVE 參數。

完整工作區 19 項測試與 Clippy 已通過，包含容器遺失時拒絕讀取保留的明文副本。另以 release CLI 使用正式 INTERACTIVE 參數，完成兩組密碼註冊、備份、結束程序後以第二組密碼重新解鎖、讀回兩台測試主機及再鎖定；原明文檔 hash 保持不變。該測試的主容器與可獨立解鎖的備份各為 1,060,892 bytes。建立與鎖定／解鎖畫面也以原生視窗擷取確認排版。

## 一手來源

- [上游固定版本與授權](https://github.com/PentHertz/LUKSbox/tree/69448f708f1328d672deb595e359579d630cb0b0)
- [上游風險說明](https://github.com/PentHertz/LUKSbox/blob/69448f708f1328d672deb595e359579d630cb0b0/DISCLAIMER.md)
- [Container API](https://github.com/PentHertz/LUKSbox/blob/69448f708f1328d672deb595e359579d630cb0b0/crates/luksbox-format/src/container.rs)
- [VFS 與儲存提交](https://github.com/PentHertz/LUKSbox/blob/69448f708f1328d672deb595e359579d630cb0b0/crates/luksbox-vfs/src/vfs.rs)
- [密碼 KDF 參數](https://github.com/PentHertz/LUKSbox/blob/69448f708f1328d672deb595e359579d630cb0b0/crates/luksbox-core/src/kdf.rs)
