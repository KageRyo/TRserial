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

## 原生桌面版

GUI 分支從已合併的 PR #1（`bee7492`）開始。使用 GPUI Kit `0.9.0`、GPUI `0.3.11` 與 gpui-tray `0.3.0`，依賴鎖定於 `Cargo.lock`。未建立 tag、GitHub Release 或安裝包，Cargo 版本未變更。

本機環境為 Linux x86_64／WSL2、Rust／Cargo 1.97.1。GUI 使用 WSLg X11 與 Mesa 軟體 Vulkan 實際開窗，五個頁面另以 GPUI 原生 renderer 檢查明暗主題與繁體中文字型。建置所需函式庫與字型解開於使用者 cache，未修改系統套件。

| 本機檢查 | 結果 |
| --- | --- |
| `cargo fmt --all --check` | 通過 |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | 通過，CLI 不使用 GUI 建置環境 |
| `cargo test --locked --all-features` | 37 項通過，2 項瀏覽器測試另外執行 |
| `cargo test --locked --test browser_flow -- --ignored` | 2 項通過，Chrome／ChromeDriver 157.0.8103.38，本機假網站 |
| `cargo build --locked --release` | 通過 |
| `cargo clippy --locked -p trserial-desktop --all-targets --all-features -- -D warnings` | 通過 |
| `cargo test --locked -p trserial-desktop -- --include-ignored` | 11 項通過，含原生繪製與系統匣 |
| `cargo build --locked --release -p trserial-desktop` | 通過 |

### 自動化涵蓋範圍

- Core：保留既有 CLI、瀏覽器與持久化測試；新增手動多帳號／歷史確認、帳號停用再啟用、重新加入的歷史基準、別名與紀錄識別、設定驗證、mock 隔離及人工重試。
- GUI：鍵盤填寫帳號表單、密碼欄位遮罩與資料庫不含密碼、候選批准後更新表格、mock 兌換停用、依識別碼保留表格選取、確認訊息只消費一次、重新讀取嘗試紀錄、背景啟停、重啟後不重送未知結果，以及單一實例鎖。
- Linux 原生程序：獨立 D-Bus fixture 接收真正的 StatusNotifierItem 註冊，呼叫 Activate 還原視窗，檢查隱藏時背景仍在執行、結束後鎖可取得及嘗試紀錄為零。另測試沒有可用 host 時保留主視窗。
- GUI `--smoke-test`：全新資料目錄、測試帳號、mock 訊息、五頁實際繪製、導覽不增加視窗、啟動／暫停／停止與正常結束。未完成測試時回傳失敗。

GUI CI 在 Ubuntu 與 Windows 分別執行嚴格 Clippy、一般測試、原生程序 smoke test 與最佳化編譯。CLI 的 Windows／Linux 測試與本機假網站瀏覽器整合測試保留。一般 CLI 建置不需 GUI 系統函式庫。

### GitHub Actions 結果

2026-11-07，PR #2 提交 `43af45a` 由 push 與 pull_request 各觸發一次 CI，兩次全部通過。SonarCloud 新增問題為 0。

| 項目 | 結果 |
| --- | --- |
| `rust`（Ubuntu／Windows） | 嚴格 Clippy、37 項一般測試、2 項本機假網站瀏覽器測試、全功能測試與 release build 通過 |
| `desktop`（Ubuntu） | 9 項 GUI 測試通過。`native_tray` 在 Xvfb 與測試用 D-Bus 上完成 StatusNotifierItem 註冊、以 Activate 還原視窗，沒有 host 時保留主視窗 |
| `desktop`（Windows） | 9 項 GUI 測試通過。原生 smoke test 偵測到系統匣後收起視窗，確認背景工作持續，再以顯示動作還原；五頁繪製、背景啟停與正常結束也完成 |

`capture_all_pages_with_native_renderer` 需要實際 Vulkan renderer，CI 不執行，只在本機檢查。Windows 的系統匣點擊與選單沒有自動化，列在下方實機驗收。

### 桌面實機驗收

| 平台／功能 | 驗收內容 |
| --- | --- |
| Windows 10／11 桌面 | Credential Manager 新增／更新／移除、Toast、系統匣點擊與選單、登入啟動、顯示縮放 |
| Ubuntu 桌面 | 已解鎖的 Secret Service、通知、GNOME／KDE 系統匣、登入啟動與 Wayland |
| 官方網站 | 單一真實帳號與有效序號、人工 Cloudflare 驗證、網站回應與遊戲內結果 |

開發與自動測試只使用假帳號及本機假網站，沒有執行正式兌換。桌面 Discord 監聽顯示未設定，Token 與頻道編輯介面尚未提供；選用 CLI 整合保留。Linux 收起視窗後會重新建立頁面，未儲存的表單內容不保留；Windows 原生隱藏會保留頁面。表格採虛擬化顯示，目前快照會讀取完整歷史，尚未驗證大量長期紀錄的效能。
