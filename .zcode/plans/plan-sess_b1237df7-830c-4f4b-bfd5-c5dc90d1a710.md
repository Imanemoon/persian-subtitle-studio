## زمینه

برنامه قرار است متن‌باز در گیت‌هاب ریلیز شود؛ کاربر می‌خواهد «همه‌چیز اتومات» باشد: پیش‌نیازها بدون نصب دستی، هشدار Defender-exclusion قبل از ورود، میان‌بر Run-as-admin، و امضای دیجیتال. امروز برنامه ۵ ابزار بیرونی را با fallback مسیرها پیدا می‌کند (`tools/mod.rs`) و اگر نباشند فقط Missing گزارش می‌شود؛ هیچ نصاب خودکاری ندارد و هیچ code-signing ای تنظیم نشده.

**تصمیم‌های گرفته‌شده (پیشنهاد من، چون پاسخ به سؤالات نیامد):**
1. **معاونت اولین اجرا** به‌جای دانلود داخل NSIS — نصاب کوچک می‌ماند، دانلود ~۱۲۰-۱۵۰MB با نوار پیشرفت واقعی/لغو/تلاش مجدد انجام می‌شود و NSIS دست‌نخورده و بدون قالب سفارشی شکننده باقی می‌ماند.
2. **sub-rs.exe داخل نصاب باندل می‌شود** (فرض: باینری خودِ کاربر است — اگر شخص ثالث است، فقط `bundle.resources` و یک URL عوض می‌شود؛ در docs/RELEASE.md نوشته می‌شود).
3. **بدون commit لایسنس** — راهنمای انتخاب در docs/RELEASE.md.
4. **امضا**: اسکریپت + مستندات صادقانه؛ اعتماد واقعی SmartScreen فقط با گواهی ممکن است (ارزان‌ترین: Azure Trusted Signing).

## ۱) بک‌اند — ماژول `src-tauri/src/setup/` (هستهٔ Tauri-free) + `commands/setup.rs`

**وابستگی‌های جدید در Cargo.toml:** `ureq = { version = "2", features = ["rustls"] }` (دانلود بلاکینگ با progress)، `zip` (استخراج ffmpeg)، `sha2` (تأیید hash). بدون reqwest سنگین.

**جدول ToolSpec (پین‌شده با نسخه و SHA256):**
- `ffmpeg`: دانلود `https://www.gyan.dev/ffmpeg/builds/ffmpeg-8.0-essentials_build.zip` → استخراج `ffmpeg.exe`+`ffprobe.exe` به `<tools>\ffmpeg\` → tool_paths.ffmpeg/ffprobe (hash موقع پیاده‌سازی محاسبه و پین می‌شود).
- `ffsubsync`: دانلود `https://www.python.org/ftp/python/3.11.9/python-3.11.9-amd64.exe` → نصب silent per-user بدون admin (`/quiet InstallAllUsers=0 TargetDir=<tools>\python Include_pip=1 PrependPath=0 Shortcuts=0`) → `<python> -m venv <tools>\ffsubsync-venv` → `pip install ffsubsync==0.5.1` → probe `venv\Scripts\ffs.exe`. پیشرفت این مرحله phase-based و صادقانه («دانلود بسته‌ها ممکن است چند دقیقه طول بکشد») بدون درصد جعلی.
- `sevenZip`: نصاب رسمی **باندل‌شده در resources** (`7z2501-x64.exe`، ۱٫۶MB) → اجرای `/S /D=<tools>\7-Zip` — بدون دانلود.
- `subsource`: **باندل `sub-rs.exe` در resources** → pre-seed هنگام شروع app (اگر پیدا نشد، کپی از resource_dir به `<tools>\sub-rs\` + ثبت در tool_paths) — این ابزار اصلاً وارد wizard نمی‌شود.

**محل نصب ابزارها:** `<app_local_data_dir>\tools\` (per-user، بدون admin). ثبت نهایی همیشه از طریق `settings.tool_paths` + `settings::save` موجود (settings/mod.rs:318) — resolver فعلی دست‌نخورده.

**رویدادها (هم‌الگوی ops:progress):** `setup:progress {toolId, phase, receivedBytes, totalBytes, message}` و `setup:done {toolId, ok, error}`.

**commandها (wrapper نازک + هستهٔ تزریق‌پذیر مثل الگوی execute_plan_items):**
- `setup_status() -> Vec<SetupToolInfo>` — از `detect_all` موجود (tools/mod.rs:114)
- `install_tool(id)` — thread + `setup:progress`؛ لغو با `AppState.setup_cancel: Arc<AtomicBool>` جدید؛ هسته: `install_tool_core(spec, downloader: &mut dyn FnMut(url, on_chunk), tools_dir, on_progress, cancel)` — تزریق downloader برای تست بدون شبکه
- `cancel_setup()`
- `is_elevated()` — PowerShell one-liner
- `add_defender_exclusions(paths)` — اجرای PowerShell با `-Verb RunAs` (پنجرهٔ UAC برای کاربر باز می‌شود؛ در UI صادقانه توضیح داده می‌شود)؛ تلاش برای تأیید با `Get-MpPreference`، در نبودش خروجی «درخواست ارسال شد»
- `create_admin_shortcut()` — PowerShell: ساخت `.lnk` روی Desktop + ست‌کردن بیت run-as-admin (بایت ۲۱ `|= 0x20`)
- ثبت همه در `generate_handler!` (lib.rs:84)

**Settings:** فیلد `setup_completed: bool` (default false) + حفظ رفتار sanitized.

## ۲) فرانت — صفحهٔ راه‌اندازی قبل از ورود

- `PageId` جدید `"setup"` در src/types/index.ts:185؛ `App.tsx`: اگر `!settings.setupCompleted && ابزارهای حیاتی (ffmpeg/ffprobe) غایب‌اند` → route اولیه setup.
- `src/pages/SetupPage.tsx`: وضعیت هر ابزار از `useAppData().tools`؛ دکمهٔ «نصب همهٔ پیش‌نیازها» + نصب تکی؛ نوار پیشرفت واقعی از listen(`setup:progress`)؛ کارت «شتاب در Windows Defender» (استثنای پوشهٔ نصب/ابزارها پیش‌فرض + اختیاریِ انتخاب پوشهٔ رسانه‌ای با picker موجود `api.pickFolder`؛ توضیح UAC)؛ کارت «میان‌بر با Run as administrator» + توصیهٔ صادقانهٔ دیفراگ (اثر واقعی سرعت همان‌جاست)؛ دکمهٔ «ورود به برنامه» وقتی ffmpeg+ffprobe حاضر شد (بقیه اختیاری؛ اگر رد شوند، pipeline همان مسیر fail-closed فعلی را دارد).
- پایان wizard: `save_settings` با `setupCompleted: true` → دیگر تکرار نمی‌شود.

## ۳) امضا و آماده‌سازی ریلیز

- `scripts/sign-release.ps1`: یافتن signtool، امضای exe و setup.exe با env (`SIGN_CERT_THUMBPRINT` یا `SIGN_PFX`+`SIGN_PFX_PASSWORD`)، timestamp `http://timestamp.digicert.com`؛ بدون secret در repo.
- `docs/RELEASE.md`: گام‌های ریلیز گیت‌هاب، آموزش امضا (Azure Trusted Signing ~$9.99/mo به‌عنوان ارزان‌ترین مسیر اعتماد SmartScreen، گواهی OV/EV، self-signed فقط برای تست)، نکتهٔ لایسنس (ffmpeg دانلود-in-time است پس اجبار GPL باندل نداریم)، provenance باندل‌ها (sub-rs مال کاربر، 7-Zip LGPL).
- tauri.conf.json: فقط `bundle.resources` برای دو فایل باندلی — قالب NSIS دست نمی‌خورد.

## ۴) تست

- تست هسته با downloader فیک (بدون شبکه): استخراج zip ساختگی، ثبت tool_paths، لغو وسط دانلود، hash نامعتبر → خطای صادقانه.
- `#[ignore]` live: دانلود واقعی ffmpeg و ساخت venv ffsubsync (شبکه‌محور، فقط دستی).
- GUI-e2e با همان روش CDP امروز: صفحهٔ setup روی ماشین فعلی (همه ابزارها هست → باید دکمهٔ «ورود به برنامه» مستقیم ظاهر شود؛ مسیر missing با پاک‌کردن موقت tool_paths).
- suite موجود: `cargo test --lib` + integration + clippy + fmt + `npm run build` + نصاب NSIS.

## فایل‌های کلیدی

- جدید: `src-tauri/src/setup/mod.rs`، `src-tauri/src/commands/setup.rs`، `src/pages/SetupPage.tsx`، `scripts/sign-release.ps1`، `docs/RELEASE.md`، `src-tauri/resources/` (sub-rs.exe + 7z installer)
- ویرایش: `Cargo.toml`، `tauri.conf.json`، `lib.rs` (commandها + pre-seed باندل‌ها + state جدید)، `state.rs` (setup_cancel)، `settings/mod.rs` (setup_completed)، `types/index.ts` + `App.tsx` + `lib/api.ts`، `hooks/useAppData.tsx`
- بازاستفاده: `resolve_tool_path`/`detect_all` (tools/mod.rs)، الگوی thread+event (subtitle.rs:581)، `save` اتمیک settings (settings/mod.rs:318)