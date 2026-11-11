# TRserial

TRserial 是「跑Online」（TalesRunner）的多帳號序號兌換工具，使用 Rust 撰寫。可以匯入序號訊息、保留獎勵描述，並透過 Chrome 的官方表單依序處理各帳號。每個帳號的結果分開儲存，重開程式後可接續尚未送出的工作。

桌面版使用原生 Rust／GPUI Kit，提供帳號、序號、兌換紀錄與系統匣操作；也保留 CLI。Discord 為選用功能，執行時無需 Python 或 Node.js。專案採用 [MIT](LICENSE) 授權。

專案尚未正式發布，以下指令從原始碼安裝。Cargo 套件版本僅供開發使用。

## 目前能做什麼

- 從手動文字、UTF-8 檔案或假訊息來源擷取多組不同長度的序號。保留原文、來源、獎勵原文及到期日文字。
- 管理多個帳號與別名。Windows 使用 Credential Manager，Linux 使用 Secret Service；SQLite 與設定檔不儲存密碼。
- 使用 Chrome／Chromium 與 WebDriver 填寫官方兌換表單。遇到驗證時等待使用者在瀏覽器操作，通過後繼續。
- 以 SQLite 記錄每個帳號／序號的工作、嘗試次數、結果、錯誤類別與時間。只有送出前的暫時錯誤會自動重試。
- 提供成功、人工驗證、失敗或結果不明，以及整批處理摘要的桌面通知。
- 用 `daemon` 接續佇列。剪貼簿監看與 Discord Bot 需額外編譯並明確啟用。

測試結果與待驗收項目見 [驗證紀錄](VALIDATION.md)。

## 安裝桌面版

使用目前的 [Rust stable](https://rustup.rs/)；桌面版至少需要 Rust 1.92。`Cargo.lock` 已納入版本控制，SQLite 隨程式編譯。兌換需要另外安裝 Chrome／Chromium 與相符版本的 ChromeDriver。

### Windows 10／11 x64

安裝 Rust 與 Visual Studio Build Tools 的「使用 C++ 的桌面開發」，在 PowerShell 執行：

```powershell
git clone https://github.com/KageRyo/TRserial.git
cd TRserial
cargo install --locked --path desktop
trserial-desktop
```

若找不到指令，重新開啟終端，或執行 `$env:USERPROFILE\.cargo\bin\trserial-desktop.exe`。

### Ubuntu／Linux x86_64

```bash
sudo apt update
sudo apt install build-essential pkg-config cmake libfontconfig-dev libwayland-dev \
  libxkbcommon-x11-dev libx11-xcb-dev libssl-dev libzstd-dev libvulkan1 \
  mesa-vulkan-drivers fonts-noto-cjk
# 依 https://rustup.rs/ 安裝 Rust stable
git clone https://github.com/KageRyo/TRserial.git
cd TRserial
cargo install --locked --path desktop
trserial-desktop
```

需要可用的 X11／Wayland 桌面與 Vulkan。密碼儲存需要登入中的 D-Bus 與已解鎖的 Secret Service，例如 GNOME Keyring。系統匣使用 StatusNotifierItem；GNOME 可搭配 AppIndicator／KStatusNotifierItem 擴充套件。偵測不到系統匣時會在畫面提示，關閉視窗會結束程式。

從 checkout 直接啟動：

```bash
cargo run --locked -p trserial-desktop
# 另用資料目錄，不影響既有帳號：
cargo run --locked -p trserial-desktop -- --data-dir .trserial/desktop
```

## 桌面版操作

1. **先做離線測試**：使用下方 `--smoke-test` 指令。測試會開啟五個頁面、建立假帳號、匯入測試訊息、啟停背景工作，再自動結束。它只接受全新資料目錄，不讀取密碼、不連線兌換網站，也不修改登入啟動設定。
2. **設定瀏覽器**：在「設定」填入 ChromeDriver 路徑，勾選「由 TRserial 管理 ChromeDriver」並儲存。Chrome 路徑可留白使用系統安裝。按「檢查 Chrome 與官方表單」，確認瀏覽器與欄位定位；檢查不填帳密、不送出表單。
3. **新增帳號**：在「帳號」填入別名、遊戲帳號與密碼。密碼欄位遮罩，密碼存入 Windows Credential Manager／Linux Secret Service。可更新密碼、改別名、移除或關閉自動處理。CLI 的 `--session-only` 帳號需先在帳號頁更新密碼，才能在桌面背景處理。
4. **匯入序號**：在「序號」輸入一組序號，或按「匯入訊息」貼上文字。列表顯示原訊息的獎勵與到期文字；待確認的候選序號需先批准。選取序號與帳號後，確認視窗會列出實際兌換範圍。正式驗收先選一個帳號及一組有效序號。
5. **核對結果**：「兌換紀錄」可依狀態、帳號與序號篩選，查看每次嘗試與結果。`unknown` 表示網站結果尚未確認；核對官方或遊戲紀錄後，再按「人工重試」。完成的組合會保留，不會因重開程式重送。
6. **啟用背景工作**：按「開始自動處理」。首次開啟或新增帳號時建立歷史基準，既有序號不會整批自動送出。「補抓歷史序號」需另外確認；重新啟用帳號也會重設基準，保留停用期間累積的序號供手動處理。

```powershell
# Windows PowerShell；每次測試使用新的目錄名稱
cargo run --locked -p trserial-desktop -- --data-dir .trserial/gui-smoke-win --smoke-test
```

```bash
# Linux 桌面
cargo run --locked -p trserial-desktop -- --data-dir .trserial/gui-smoke-linux --smoke-test
```

「暫停」與「停止」會取消目前工作並關閉瀏覽器，已進入送出階段的結果保留為 `unknown`。暫停後可以手動兌換或編輯設定；再次開始會接續符合規則的待處理工作。遇到 Cloudflare 驗證時 Chrome 保持可見，請在瀏覽器完成驗證。

系統匣可顯示／收起主視窗、開始／暫停、開啟設定與結束程式。有系統匣時，關閉視窗會保留背景工作；選「結束」才會關閉瀏覽器並釋放程序鎖。Linux 若失去系統匣 host，會重新開啟視窗。Linux 收起視窗會清除尚未送出或儲存的表單內容，請先儲存設定。每個資料目錄只允許一個桌面程序，CLI 與桌面共用工作鎖。

「設定」可儲存明暗外觀、通知、開啟時自動處理與登入啟動。登入啟動僅寫入目前使用者的 Windows Run 登錄值或 Linux `~/.config/autostart/trserial.desktop`。桌面會顯示執行狀態、目前序號／帳號與驗證等待；新確認序號與兌換結果使用既有桌面通知。

桌面版的 Discord 監聽顯示「未設定」，目前未提供 Token 設定介面；既有頻道 ID 僅供查看。需要 Discord 時使用下方選用 CLI 功能，Token 仍由環境變數提供。

## 安裝 CLI

使用目前的 Rust stable（edition 2024）。`Cargo.lock` 已納入版本控制。SQLite 隨程式編譯，不必另裝資料庫。

### Windows 10／11 x64

安裝 [Rust](https://rustup.rs/) 與 Visual Studio Build Tools 的「使用 C++ 的桌面開發」，再在 PowerShell 執行：

```powershell
git clone https://github.com/KageRyo/TRserial.git
cd TRserial
cargo install --path . --locked
trserial init
trserial doctor
```

若 `trserial` 找不到，重新開啟終端，或使用 `$env:USERPROFILE\.cargo\bin\trserial.exe`。

### Ubuntu／Linux x86_64

```bash
sudo apt install build-essential pkg-config
# 先依 https://rustup.rs/ 安裝 Rust stable
git clone https://github.com/KageRyo/TRserial.git
cd TRserial
cargo install --path . --locked
trserial init
trserial doctor
```

Linux 憑證庫需要登入中的 D-Bus 工作階段與已解鎖的 Secret Service，例如 GNOME Keyring 或提供 Secret Service 的 KWallet。D-Bus 建置來源隨程式提供。通知需要桌面通知服務；人工驗證需要可見的瀏覽器。WSL、SSH 或純命令列環境可用 `--session-only` 改為執行時輸入密碼。

## 設定 Chrome 與 WebDriver

安裝 Chrome／Chromium，再從 [Chrome for Testing](https://googlechromelabs.github.io/chrome-for-testing/) 取得**相符版本**的 ChromeDriver。將 `chromedriver`／`chromedriver.exe` 放入 PATH，先在另一個終端啟動：

```bash
chromedriver --port=9515
```

這是外部管理模式，保持這個終端執行即可。`webdriver_url` 也可設定外部服務使用的路徑前綴，例如 `/wd/hub`。瀏覽器與驅動程式需自行安裝。

也可讓 TRserial 管理 ChromeDriver：

```bash
trserial --managed-driver browser-check
# 不在 PATH 時指定執行檔：
trserial --driver-binary /absolute/path/to/chromedriver browser-check
```

管理模式先檢查本機 `/status`，已有可用的 WebDriver 就沿用，否則啟動設定的執行檔。啟動時採 ChromeDriver 預設的 loopback 綁定，並以同一使用者共用的端點鎖防止重複啟動。程式結束時會回收自己啟動的程序，外部程序保留；子程序中斷後，下次準備瀏覽器時會重新啟動。啟動失敗與等待就緒都有時間上限。

`--managed-driver` 與 `--driver-binary` 只影響當次執行。若要固定使用管理模式，可在 `init` 顯示的設定檔加入：

```toml
[driver]
managed = true
binary = "/absolute/path/to/chromedriver"
startup_timeout_seconds = 15
```

Windows 的 `binary` 可寫成 `"C:/Tools/chromedriver.exe"`。管理模式使用 `http://127.0.0.1:9515`，可在 `webdriver_url` 改用其他本機連接埠。

`trserial init` 會顯示設定檔位置。可參照 [config.example.toml](config.example.toml)，或明確指定：

```bash
trserial --config config.example.toml doctor
```

預設為有視窗模式。`browser_binary` 可指定 Chrome 執行檔，`headless = true` 只適合不需人工驗證的環境。官方目前使用 Cloudflare Turnstile，建議保持 `headless = false`。背景工作也需要已登入的圖形桌面與可用的 WebDriver。

程式會檢查官方網域、表單目的地與欄位內容。欄位為 `account`、`passward`（官方拼字）、`code`，送出按鈕為 `#btnSend`。

## 第一次使用：離線 smoke test

先在獨立資料目錄試跑，測試帳號只存中繼資料：

```bash
trserial --data-dir .trserial/smoke init
trserial --data-dir .trserial/smoke account add-test smoke
trserial --data-dir .trserial/smoke ingest --mock examples/messages.json
trserial --data-dir .trserial/smoke redeem malojam1106 --account smoke --dry-run
```

這四步只在本機執行。Dry run 會檢查設定、序號格式、指定帳號及既有工作狀態，列出預計工作並保留佇列原狀。測試帳號與 mock 序號都帶有 `test_only` 標記。

也可在另一個全新目錄用單一指令建立測試帳號、匯入範例並執行 dry run：

```bash
trserial --data-dir .trserial/quick-smoke smoke-test
```

接著檢查 Chrome 啟動與官方表單。這一步會開啟官方網站，檢查轉址、表單目的地、欄位和驗證控制項，然後關閉瀏覽器；帳密與序號欄位保持原狀：

```bash
trserial --data-dir .trserial/smoke --managed-driver browser-check
```

檢查結果會列出缺少的定位與變更的欄位類型，或指出啟動失敗、轉址及驗證中介頁。`doctor` 可另外查看 `/status`、管理模式與驅動程式路徑。

## 準備好後，以單一帳號兌換

正式帳號使用預設的使用者資料目錄，與 smoke test 目錄分開：

```bash
trserial init
trserial account add
# 依序輸入別名、遊戲帳號、密碼；密碼不回顯。
trserial account list
```

選定一組真正要兌換的序號後，以 `YOUR_CODE`、`YOUR_ALIAS` 代入：

```bash
trserial ingest --text "序號: YOUR_CODE"
trserial redeem YOUR_CODE --account YOUR_ALIAS --dry-run
trserial --managed-driver redeem YOUR_CODE --account YOUR_ALIAS
```

新增帳號需要可用的系統憑證庫。若要每次執行才輸入密碼：

```bash
trserial account add --session-only
```

密碼讀取失敗時，互動終端會重新詢問密碼；背景工作則記為 `failed`，修復後可重試。帳號設定與兌換歷史分開儲存。移除帳號後，歷史仍會保留；在同一資料目錄重新新增完全相同的遊戲帳號，會接回原本的兌換紀錄。

```bash
trserial account remove my-alias
```

## 匯入、檢查與兌換

```bash
trserial ingest --text "序號: malojam1106"
trserial ingest --file examples/message.txt
trserial ingest --mock examples/messages.json
trserial code list
trserial history --messages
```

有「序號」「兌換碼」「Code」等標記的內容會視為已確認；獨立出現的英數混合字串會標為 `needs_review`，確認後才能排程：

```bash
trserial code approve AbC123
trserial redeem AbC123 --account my-alias --dry-run
trserial redeem AbC123 --account my-alias
trserial redeem malojam1106 --all
```

序號以原始大小寫儲存。純英文或純數字序號請用 `序號: ...` 匯入。到期日保留原文，過期狀態依網站回覆判定。獎勵描述接在最近一組序號後；一則訊息有多組序號時，可用 `history --messages` 核對原文。

出現驗證時，在瀏覽器完成驗證，程式便會接續送出。等待期間可用 Ctrl+C 結束，未送出的工作留待下次處理。

```bash
trserial history
trserial history --attempts
trserial status
trserial daemon
```

單次 `redeem` 只處理一輪，暫時失敗的工作會依設定排定下次時間，由 `daemon` 接續。

每個帳號首次執行 daemon 時，會將當時已有的序號記為歷史基準。設定好帳號後，可先執行一次 `trserial daemon --once` 建立基準，再開始匯入新訊息。之後首次發現且已確認的序號才會自動排入；daemon 離線期間新增的序號也會在下次啟動時處理。重貼舊訊息或確認舊候選不會改變發現順序，新加入的帳號也有自己的基準。

若要補抓歷史序號，請明確指定：

```bash
trserial --managed-driver daemon --backfill --once
```

升級前已存在的待處理工作會保留，預設留作 `historical`。可用 `history` 查看 `manual`、`new`、`historical` 或 `test_only` 分類，再以 `redeem CODE --account ALIAS` 選定工作，或用 `--backfill` 整批排入。成功、已兌換及過期的帳號／序號組合仍保持完成狀態。

| 狀態 | 意義與後續處理 |
| --- | --- |
| `pending` | 尚未送出，或等候有上限的退避重試 |
| `processing` | 正在準備或等待網站回覆 |
| `waiting_verification` | 等待在瀏覽器完成驗證 |
| `success` | 網站確認兌換成功 |
| `already_redeemed` | 網站回覆已兌換 |
| `expired` | 網站回覆已過期 |
| `failed` | 網站拒絕、驗證逾時、無法取用密碼或送出前重試已達上限 |
| `unknown` | 已進入送出階段，結果待核對 |

程式會在點擊前記錄送出狀態。進入送出階段後若中斷或收不到可辨識的回覆，工作會標為 `unknown`。同一帳號／序號組合只建立一份工作。核對結果後，可將失敗或結果不明的工作排回佇列：

```bash
trserial retry 1 --acknowledge
trserial daemon --once
```

`retry` 接受 `failed`／`unknown`，會清除重試計數並開始新一輪嘗試。`success`、`already_redeemed` 與 `expired` 保留為完成狀態。

## 桌面通知與背景啟動

```bash
trserial doctor --test-notification
```

Windows 使用 Toast 通知，Linux 使用桌面通知服務。通知顯示網站結果，獎勵描述來自匯入訊息。顯示設定由作業系統控制，完整結果可查 `history`。可在設定中關閉 `notifications`。

Linux 可安裝使用者服務範例：

```bash
mkdir -p ~/.config/systemd/user
cp examples/trserial.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now trserial.service
journalctl --user -u trserial.service
```

範例服務會啟用 managed driver。先在該使用者的設定檔中將 `driver.binary` 設為 ChromeDriver 絕對路徑。服務需要 `~/.cargo/bin/trserial`、桌面 D-Bus／顯示環境與 ChromeDriver。登入後可用 `systemctl --user import-environment DISPLAY WAYLAND_DISPLAY XAUTHORITY` 匯入圖形工作階段環境；停止服務用 `systemctl --user stop trserial`。

Windows 可在已登入的 PowerShell 執行 [工作排程範例](examples/windows-task.ps1)：

```powershell
.\examples\windows-task.ps1
# 取消自動啟動：
Unregister-ScheduledTask -TaskName TRserial -Confirm:$false
```

工作會在使用者登入後，以一般權限執行。範例排程會啟用 managed driver；請在設定檔的 `driver.binary` 填入 ChromeDriver 絕對路徑。背景工作使用系統憑證庫中的密碼。新增、移除帳號或人工重試前，先停止 daemon。

### 選用剪貼簿監看

```bash
cargo install --path . --locked --features clipboard
trserial daemon --clipboard
```

加上 `--clipboard` 後，每輪會在本機檢查剪貼簿，儲存含候選序號的訊息原文。不確定的候選仍需人工確認。Linux 使用 X11，Wayland 需搭配 XWayland。

## 日後啟用 Discord

Discord 預設關閉。可先用假訊息試跑匯入流程：

```bash
trserial ingest --mock examples/messages.json
trserial daemon --mock examples/messages.json --once
```

兩種 mock 指令都只匯入測試資料；`daemon --mock` 匯入後就結束，既有佇列保持原狀。mock 標記會保存在資料庫，即使之後重貼同一序號，仍維持 `test_only`。測試帳號與 mock 序號可用於 dry run 或本機假網頁測試，正式瀏覽器兌換會拒絕這些資料。

待要連接 Discord 時：

1. 在 Discord Developer Portal 建立官方 Bot，啟用 **Message Content Intent**。
2. 邀請到目標伺服器，授予目標頻道的 View Channel、Read Message History 權限。
3. 以 `cargo install --path . --locked --features discord` 編譯。
4. 將 Token 設為執行環境的 `TRSERIAL_DISCORD_TOKEN`。
5. 在設定中將 `[discord] enabled = true`，並填入 `channels = [實際頻道的數字ID]`，再執行 `trserial daemon`。

Serenity 介接器會接收新訊息，使用相同解析器與去重邏輯。第一次啟用只讀取各頻道最近 100 則，之後根據 SQLite 游標分頁補抓；重新建立連線與恢復連線時也會補抓。游標只在訊息匯入完成後前進。補抓範圍為首次啟用時最近 100 則，以及游標之後仍存在的訊息原文。

## 資料與安全

預設資料位置由作業系統決定，`init` 會列出實際路徑。Windows 通常為 `%LOCALAPPDATA%\trserial\data`，Linux 為 `$XDG_DATA_HOME/trserial`（未設定時為 `~/.local/share/trserial`）。所有指令都可加 `--data-dir PATH`。

- `config.toml`：操作設定，不放密碼或 Token。
- `desktop.toml`：桌面外觀與啟動偏好。
- `desktop.lock`：同一資料目錄的桌面程序鎖。
- `accounts.sqlite`：帳號識別碼、別名、使用者名稱、憑證模式與自動處理開關。與兌換歷史分開管理。
- `history.sqlite`：來源原文、解析獎勵、序號、每帳號工作、嘗試紀錄及 Discord 游標。
- `worker.lock`：防止同一資料目錄有多個瀏覽器工作同時執行；離開時由作業系統釋放。

桌面版會更新帳號資料格式。同時使用 CLI 時，請從同一份原始碼更新 CLI；舊執行檔無法讀取升級後的帳號資料。

資料庫未加密，內含帳號名稱／別名與匯入訊息，請保護資料目錄與備份。Linux 建立目錄時設為僅本人可存取；Windows 使用使用者目錄既有權限。密碼只放系統憑證庫或記憶體。錯誤紀錄使用錯誤類別與表單定位，轉址訊息只顯示網域，不回顯查詢參數或網站原始回應。

瀏覽器使用 WebDriver 的暫存設定檔。ChromeDriver 被強制終止後，殘留的 Chrome 視窗可手動關閉；已進入送出階段的工作仍會記為 `unknown`。`.gitignore` 已排除 `.env`、SQLite、本機狀態及指定的瀏覽器設定檔目錄。

## 疑難排解

- **桌面視窗無法開啟**：Linux 確認登入 X11／Wayland 與 Vulkan 驅動；純 SSH 環境請使用 CLI。啟動錯誤會輸出到終端。
- **關閉後找不到視窗**：從系統匣點選圖示或「顯示視窗」；沒有可用系統匣時程式會保留視窗，關閉則結束。
- **帳號頁無法儲存密碼**：解鎖作業系統憑證庫後重試，桌面版不會改存明文或要求在隱藏終端輸入。
- **無法連線 WebDriver**：執行 `doctor` 查看 `/status`；外部管理模式先啟動 ChromeDriver，管理模式則檢查 `driver.binary` 與執行權限。版本可用 `chrome --version` 與 `chromedriver --version` 核對。
- **憑證庫不可用**：解鎖系統憑證庫／確認 Linux Secret Service；或用 `account add --session-only` 並在互動終端執行。
- **一直等待驗證**：在瀏覽器完成驗證；headless 模式請改回有視窗模式。
- **`unknown`**：核對遊戲或官方結果後，需要重試時使用 `retry ID --acknowledge`。
- **欄位或送出目的地變更**：先執行 `browser-check` 取得缺少的定位，再更新 `src/browser.rs` 與測試。
- **已有程序執行中**：停止同資料目錄的 daemon 後再執行。

## 開發與驗證

```bash
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release
# 額外檢查可選來源：
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
# 桌面版（Linux 需先安裝上述原生函式庫）：
cargo clippy --locked -p trserial-desktop --all-targets --all-features -- -D warnings
cargo test --locked -p trserial-desktop
cargo build --locked --release -p trserial-desktop
```

一般測試不需帳號、網路、桌面憑證庫或 Discord。真實瀏覽器測試使用本機 HTTP 假頁面，需相符版本的 Chrome／ChromeDriver：

```bash
# Linux 範例；Windows PowerShell 以 $env:CHROME_BIN / $env:CHROMEDRIVER 設定。
export CHROME_BIN=/absolute/path/to/chrome
export CHROMEDRIVER=/absolute/path/to/chromedriver
cargo test --locked --test browser_flow -- --ignored
```

[GitHub Actions](https://github.com/KageRyo/TRserial/actions) 分別在 Ubuntu 與 Windows 驗證 CLI 與桌面版，包含 Clippy、單元／介面測試、原生視窗 smoke test 及最佳化編譯。Linux 另以獨立 D-Bus fixture 測試實際系統匣的隱藏、還原與結束。`cargo build --release` 只做本機編譯。一般 CI 不產生發布檔；draft release 由手動觸發的 workflow 建立，只會產生 draft，流程見 [發布說明](docs/releasing.md)。

`desktop/` 負責 GPUI Kit 頁面與系統匣，透過背景執行緒的命令／快照更新 UI。`src/service.rs` 共用帳號、匯入、確認範圍、設定與瀏覽器工作生命週期；SQL 與兌換規則留在 core。Workspace 預設只建置 CLI，不要求 GUI 的系統函式庫。
