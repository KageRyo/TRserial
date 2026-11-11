# 發布 TRserial

發布檔由 GitHub Actions 建置。workflow 只會建立 draft，要不要發布、什麼時候發布由擁有者決定。

## 流程

從 `main` 觸發：

```sh
gh workflow run release.yml --ref main -f tag=v0.1.0
```

tag 必須是 `vX.Y.Z` 或 `vX.Y.Z-prerelease`，並且等於 `Cargo.toml` 與 `desktop/Cargo.toml` 的版本。含 `-` 的 tag 會標成 pre-release。

workflow 依序執行：

1. 在 Windows 與 Linux 編譯 CLI 和桌面版，對編譯結果跑原生 smoke test 後打包。Windows 版內嵌圖示與版本資訊，並用 Inno Setup 產生安裝程式。
2. 產生 `SHA256SUMS` 與 `RELEASE-METADATA.json`。
3. 在全新的 Windows 與 Linux runner 下載打包檔，核對檢查碼與 metadata，執行 CLI `--version` 和桌面 smoke test。Windows 另外會靜默安裝、執行安裝好的程式，再解除安裝。
4. 為每個檔案建立 GitHub Artifact Attestations，並逐一驗證。
5. 前面全部通過才建立或更新 draft。同一個 tag 重跑時，會更新 draft 的 target、說明與檔案，並刪掉這次沒有產生的舊檔；已發布的 release 不會被修改。

修改 workflow、安裝腳本或圖示相關檔案的 PR 會跑到第 3 步，不會建立 release。

## 發布檔

| 檔案 | 內容 |
| --- | --- |
| `trserial-windows-amd64-setup.exe` | Windows 安裝程式 |
| `trserial-windows-amd64-portable.exe` | Windows 桌面版單一執行檔 |
| `trserial-windows-amd64.zip` | Windows 桌面版、CLI、README、LICENSE 與設定範例 |
| `trserial-linux-amd64.tar.gz` | Linux 桌面版、CLI、README、LICENSE 與設定範例 |
| `SHA256SUMS` | 上面四個檔案的 SHA-256 |
| `RELEASE-METADATA.json` | tag、來源提交、Windows 簽章狀態與驗證方式 |

檔名不含版本，`https://github.com/KageRyo/TRserial/releases/latest/download/<檔名>` 會指向最新發布的版本。

驗證下載的檔案：

```sh
gh attestation verify trserial-windows-amd64-setup.exe --repo KageRyo/TRserial
```

## 安裝程式

腳本是 `desktop/installer/trserial.iss`。程式安裝在 `%LOCALAPPDATA%\Programs\TRserial`，不需要系統管理員權限。安裝前會顯示 MIT 授權並要求同意，介面可選英文或正體中文，語系檔的來源見 `desktop/installer/languages/README.md`。

安裝會建立開始選單捷徑，桌面捷徑可選。解除安裝會移除程式、捷徑與登入啟動的登錄值，保留使用者資料目錄裡的設定與兌換紀錄，以及憑證庫裡的密碼。

`AppId` 不能更改，Windows 靠它辨識同一個程式來覆蓋安裝與解除安裝。

## 自動更新

桌面版用 GitHub Releases API 找比目前版本新的已發布版本，略過 draft；目前不是預覽版時也略過 pre-release。Windows 安裝版下載 `trserial-windows-amd64-setup.exe`，Linux 下載 `trserial-linux-amd64.tar.gz`，兩者都必須在 `SHA256SUMS` 找到相符的檢查碼才會安裝，下載網址也只接受 GitHub 的 HTTPS 網域。

所以這兩個檔名與 `SHA256SUMS` 不能改名或缺少。缺少時桌面版只會引導使用者到下載頁，不會自動安裝。Windows 免安裝版一律開啟下載頁，因為安裝程式會裝到另一個位置。

## 程式碼簽章

目前沒有 Authenticode 憑證，Windows 檔案都未簽章。workflow 會用 `Get-AuthenticodeSignature` 確認，`RELEASE-METADATA.json` 和 release 說明也會標示未簽章，使用者第一次開啟時可能看到 SmartScreen。

日後取得憑證（例如 [SignPath Foundation](https://signpath.org/) 的開源專案簽章）時，在打包前簽署兩個 exe 與安裝程式，並把 metadata 改成 `signed`。憑證與密碼只能放在 repository secrets，不能進 Git。

## 發布前檢查

按下 Publish 前，在實機確認：

- 安裝程式顯示 MIT 授權，未同意就不能繼續；英文與正體中文都能完成安裝，語言名稱顯示「正體中文」
- 雙擊桌面版時沒有主控台視窗
- exe、安裝程式、開始選單與桌面捷徑都顯示 TRserial 圖示
- 安裝版、免安裝版與 zip 的版本相同
- 解除安裝後程式與捷徑都已移除，帳號與兌換紀錄還在
- Linux 壓縮檔在依 README 安裝函式庫後能開啟
- `VALIDATION.md` 的「桌面實機驗收」已完成，未完成的項目寫進 release 說明
- release 說明標示未簽章
- 從上一個已發布版本自動更新到這一版：Windows 安裝版與 Linux 都能完成並重新開啟；第一個版本之前沒有可更新的舊版，從下一版開始檢查
