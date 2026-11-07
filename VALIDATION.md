# 驗證紀錄

## 本機檢查

2026-11-06 於 Linux x86_64（Ubuntu 26.04／WSL）執行，使用 Rust 1.97.1 與 Cargo 1.97.1。

| 指令 | 結果 |
| --- | --- |
| `cargo fmt --check` | 通過 |
| `cargo clippy --locked --all-targets -- -D warnings` | 通過 |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | 通過 |
| `cargo test --locked` | 18 個測試通過 |
| `cargo test --locked --all-features` | 18 個測試通過 |
| `cargo test --locked --test browser_flow -- --ignored` | 1 個瀏覽器測試通過 |
| `cargo build --locked --release` | 通過 |

一般測試包含 6 個解析與回應分類測試、2 個 CLI 流程測試、9 個持久化與工作流程測試，以及 1 個憑證庫故障測試。涵蓋訊息格式、去重、人工確認、每帳號狀態、重啟復原、退避重試、取消、驗證等待與逾時、通知事件、帳號重新加入及 dry run。

端到端測試以假訊息、兩個假帳號及假 Redeemer，串接解析器、SQLite、工作佇列與通知介面。憑證測試會檢查資料檔，並模擬憑證庫故障；CLI 測試會檢查設定錯誤的輸出內容。

瀏覽器測試使用 Chrome for Testing 155.0.8059.12 與相符的 ChromeDriver。本機 HTTP 網頁會接收表單，檢查欄位內容與送出次數。測試涵蓋驗證等待、欄位截斷、跨網域表單拒絕，以及成功、已兌換、過期、無效和未知回應。

測試環境的 Chrome 函式庫解開於 `/tmp/trserial-browser-deps`，以 `LD_LIBRARY_PATH` 載入。容器中的啟動腳本使用 `--no-sandbox`；應用程式採 Chrome 預設安全設定。

release 執行檔另測過互動新增假帳號、匯入 `examples/messages.json`、dry run、`doctor` 與 `status`。重複訊息產生 1 組序號，dry run 後工作與嘗試紀錄皆為零。

## 官方表單

2026-11-06 檢視的 [官方序號表單](https://trevent.funtown.com.hk/serial/app/serial_code.php) 使用以下欄位：

| 項目 | 定位或設定 |
| --- | --- |
| 表單 | `form#serial` |
| 送出目的地 | POST `/serial/app/serial_code_handler.php` |
| 帳號／密碼／序號 | `account`／`passward`／`code`，各限 30 字元 |
| 送出按鈕 | `#btnSend` |
| 驗證 | Cloudflare Turnstile，欄位為 `cf-turnstile-response` |

程式依這些欄位填寫表單。欄位截斷輸入時會停止送出；網站回應以精確文字分類，其餘記為 `unknown`。

## 待驗收項目

| 環境／功能 | 待驗收內容 |
| --- | --- |
| Windows 桌面 | Credential Manager、Toast 通知、工作排程器 |
| Linux 桌面 | 已解鎖的 Secret Service、通知顯示 |
| 正式網站 | 真實帳號兌換、人工驗證、回應文字與遊戲內入帳 |
| Discord Bot | 頻道權限、新訊息、斷線恢復與歷史分頁 |
| 剪貼簿 | Windows／Linux 桌面監看 |
| 自動啟動 | systemd 使用者服務與 Windows 登入排程 |

Ubuntu 與 Windows 的 CI 結果見 [GitHub Actions](https://github.com/KageRyo/TRserial/actions)。

## 2026-11-07：桌面使用加固

本機通過格式檢查、全功能 Clippy、31 項一般測試、2 項瀏覽器／程序整合測試與 release build。

新增測試涵蓋每帳號歷史基準、重啟去重、舊版資料庫待處理工作、mock 資料隔離、測試帳號、WebDriver 狀態協定、啟動失敗、同端點並行啟動、外部程序保留及子程序中斷後重啟。唯讀檢查以本機頁面的輸入事件與 POST 計數確認沒有填寫或送出表單，也測試了欄位遺失與轉址。

在全新資料目錄新增測試帳號、匯入範例並 dry run 後，工作與嘗試紀錄皆為零。另用 managed driver 執行 `browser-check`，官方表單的五個定位與驗證控制項皆可讀取；檢查後 Chrome 與受管理的 ChromeDriver 已結束。

以 `ss` 確認 ChromeDriver 預設只監聽 `127.0.0.1` 與 `::1`。managed 模式採用這個預設，不傳入會改為全介面監聽的 `--allowed-ips`。
