# SubSync Studio

**مدیریت خودکار زیرنویس فارسی سریال‌ها و فیلم‌ها** — جایگزین گرافیکی و مدرن برای خط لوله
PowerShell + FFmpeg + SubSource CLI + 7-Zip + FFsubsync.

> نرم‌افزار دسکتاپ ویندوزی ساخته‌شده با **Tauri 2 + Rust + React + TypeScript + Tailwind CSS**.
> ریپو **خصوصی** است؛ هیچ رسانه واقعی، توکن یا اطلاعات شخصی داخل آن commit نمی‌شود.

---

## وضعیت توسعه

| فاز | شرح | وضعیت |
| --- | --- | --- |
| ۱ | Foundation — اسکن واقعی پوشه‌ها، تشخیص S/E، داشبورد، تشخیص ابزارها، تنظیمات | ✅ تکمیل |
| ۲ | Subtitle Engine — FFprobe، استخراج زیرنویس داخلی، تشخیص محتوای فارسی | ✅ تکمیل |
| ۳ | Download & Archive — SubSource، Staging با Hard Link، ZIP/RAR با 7-Zip | ✅ تکمیل |
| ۴ | Sync Engine — FFsubsync، Workerها، Stop/Retry | ⏳ بعدی |
| ۵ | Packaging — Installer ویندوز، مستندات توزیع | 🟡 نصب‌کننده آماده |

## امکانات فعلی (فاز ۲)

- **اسکن بازگشتی واقعی** پوشه ریشه با ۱۴ پسوند ویدیویی، مستثناکردن پوشه‌های تعریف‌شده
  (پیش‌فرض: `Ted Lasso` و `Better Call Saul` — قابل تغییر در تنظیمات)، پوشه‌های مخفی و
  پوشه‌های کاری `_SubSource_*`
- **تشخیص سریال/فصل/قسمت** از نام فایل (`S04E01`، `2x05`) و ساختار پوشه (`Season 02`، `فصل 1`)
- گارد ضد خطای تطبیق: `E010` هرگز `E01` حساب نمی‌شود
- **تشخیص زیرنویس کناری** هر ویدیو با نشانه‌های فارسی (fa/fas/per/farsi/persian/فارسی)
- **داشبورد RTL فارسی** با کارت‌های آماری، فیلتر جدول، لاگ زنده، صفحه پردازش
- **تشخیص ابزارهای خارجی** با پروب واقعی (`--version`/`--help`): FFmpeg، FFprobe، 7-Zip،
  SubSource CLI، FFsubsync — با امکان تعیین مسیر دستی
- **تنظیمات ماندگار** (JSON در پوشه config) با ذخیره امن (نوشتن موقت + rename)
- لغو اسکن در هر لحظه بدون خراب کردن هیچ فایلی

### موتور زیرنویس داخلی (فاز ۲)

- **تحلیل با FFprobe**: مشخصات کامل هر Subtitle Stream (Global Index، کدک، زبان،
  عنوان، Disposition پیش‌فرض/Forced) با پارس JSON و گارد خطا
- **دسته‌بندی کدک‌ها**: متنی (subrip/ass/ssa/webvtt/mov_text)، تصویری
  (PGS/VobSub/Xsub/DVB — با پیام صریح نیاز به OCR)، ناشناخته
- **تشخیص فارسی دو لایه**: اول Language Tag/عنوان Track (fa/fas/per/farsi/persian/فارسی)،
  بعد **تأیید محتوا** با رنج‌های یونیکد فارسی — Tag فارسی با متن لاتین «فارسی تأییدشده» حساب نمی‌شود
- **استخراج امن با FFmpeg**: خروجی اول فایل موقت → اعتبارسنجی (غیرخالی + ساختار cue معتبر +
  شمارش فارسی) → انتقال اتمیک به `<base>.fa.srt` **بدون هیچ بازنویسی**
- **پیش‌به‌جویی فایل‌های موجود**: `.fa.srt`، `.fa.synced.srt` و `.srt` فارسیِ موجود → Skip با دلیل
- **Extract All دو مرحله‌ای**: پلن خلاصه (آماده‌ها + ردشده‌ها با دلیل) → تأیید صریح کاربر → اجرا
- چند Track فارسی = `NeedsTrackSelection`؛ انتخاب دستی از UI — انتخاب تصادفی ممنوع
- لغو عملیات فرایند FFmpeg را می‌کشد و فایل موقت همان عملیات را پاک می‌کند

## پیش‌نیازها

### برای استفاده از نسخه نصب‌شده (کاربر نهایی)

- Windows 10/11 (64-bit)
- Microsoft Edge WebView2 Runtime — روی Windows 11 از قبل نصب است؛ روی Windows 10 هم
  معمولاً همراه Edge وجود دارد. اگر نصب نباشد، نصب‌کننده به اینترنت نیاز دارد تا Bootstrapper
  رسمی مایکروسافت را دانلود و نصب کند.
- ابزارهای خارجی زیر **داخل نصب‌کننده نیستند** و باید جداگانه نصب شوند (بخش «ابزارهای خارجی»).

### برای توسعه و ساخت installer

- [Node.js](https://nodejs.org) ≥ 22 و npm
- [Rust](https://rustup.rs) (toolchain `stable-msvc`) + MSVC Build Tools
- WebView2 (برای اجرای Tauri روی دستگاه توسعه)
- ابزارهای خارجی فقط برای تست دستی pipeline لازم‌اند

## اجرای محیط توسعه

```bash
npm install
npm run tauri dev
```

`npm run tauri dev` ابتدا Vite را روی `http://localhost:5173` بالا می‌آورد و سپس پنجره Tauri را
باز می‌کند. اجرای مستقیم `src-tauri/target/debug/persian-subtitle-studio.exe` بدون Vite باعث
خطای `localhost refused to connect` می‌شود؛ این رفتار طبیعی نسخه توسعه است.

یا فقط فرانت‌اند:

```bash
npm run dev
```

## تست‌ها و بررسی‌ها

```bash
cd src-tauri
cargo fmt
cargo check
cargo test --lib          # تست‌های واحد
cargo test --lib -- --ignored   # تست‌های زنده: تشخیص ابزارها + چرخه کامل ffmpeg روی فایل مصنوعی
cargo clippy --all-targets -- -D warnings
```

build فرانت‌اند:

```bash
npm run build
```

## ساخت نصب‌کننده ویندوز (Release)

> ⚠️ برای ساخت نسخه توزیع **فقط** از دستور زیر استفاده کنید:
>
> ```bash
> npm ci
> npm run tauri build
> ```
>
> هرگز `cargo build --release` را برای ساخت نسخه نهایی اجرا نکنید. آن دستور feature
> `custom-protocol` مربوط به Tauri را فعال نمی‌کند؛ در نتیجه فایل اجرایی به‌جای فرانت‌اند
> بسته‌شده، `http://localhost:5173` را باز می‌کند و صفحه «Hmmm… can't reach this page /
> localhost refused to connect» (با لینک «Checking the proxy and the firewall») نمایش داده
> می‌شود. این خطا ربطی به پروکسی ویندوز ندارد.

خروجی دقیق:

```text
src-tauri/target/release/bundle/nsis/SubSync Studio_0.1.0_x64-setup.exe
```

در صورت نیاز فقط NSIS:

```bash
npm run tauri build -- --bundles nsis
```

`npm run tauri build` این مراحل را انجام می‌دهد: `npm run build` (TypeScript + Vite → `dist/`)،
کامپایل Rust با assetهای جاسازی‌شده، و ساخت installer با NSIS. Node.js/npm/Rust فقط روی
دستگاه سازنده لازم است؛ کاربر نهایی به هیچ‌کدام نیاز ندارد.

### نصب

1. فایل `...-setup.exe` را اجرا کنید.
2. در صفحه «Choose Install Location» مسیر دلخواه را انتخاب کنید (پیش‌فرض:
   `%LOCALAPPDATA%\SubSync Studio`؛ بدون نیاز به Administrator).
3. در پایان می‌توانید تیک ساخت میان‌بر Desktop را بزنید؛ میان‌بر Start Menu هم ساخته می‌شود.
4. برنامه را از Start Menu یا Desktop اجرا کنید. برای باز شدن رابط کاربری به Vite، Node.js،
   اینترنت یا اجرای دستی هیچ سروری نیاز نیست.

نصب‌کننده امضای دیجیتال (Code Signing) ندارد؛ ویندوز ممکن است هشدار SmartScreen نشان دهد.
در این حالت «More info» → «Run anyway» را انتخاب کنید.

### حذف نصب

از Settings → Apps → Installed apps → SubSync Studio → Uninstall، یا فایل
`uninstall.exe` داخل پوشه نصب. در حین حذف، گزینه «Delete app data» به‌صورت پیش‌فرض
غیرفعال است؛ تنظیمات و لاگ‌ها پاک نمی‌شوند. داده‌های کاربر در این مسیرها هستند:

- `%APPDATA%\com.subsync.studio` — تنظیمات (`settings.json`)
- `%LOCALAPPDATA%\com.subsync.studio` — لاگ‌ها و کش WebView2

این مسیرها بیرون از پوشه نصب هستند و با upgrade/uninstall عادی حذف نمی‌شوند.

## ابزارهای خارجی (بسته‌بندی نمی‌شوند)

نصب‌کننده فقط `persian-subtitle-studio.exe` و `uninstall.exe` را نصب می‌کند. هیچ‌کدام از
ابزارهای زیر داخل installer نیستند:

| ابزار | کاربرد | مسیر پیش‌فرض تشخیص |
| --- | --- | --- |
| FFmpeg / FFprobe | تحلیل، استخراج و پردازش زیرنویس | `PATH` |
| 7-Zip (`7z.exe`) | باز کردن ZIP/RAR | `%ProgramFiles%\7-Zip` یا `PATH` |
| SubSource CLI (`sub-rs.exe`) | دانلود زیرنویس | `%LOCALAPPDATA%\Programs\SubSource` یا `PATH` |
| FFsubsync (`ffs.exe`) | هم‌زمان‌سازی زیرنویس | `%LOCALAPPDATA%\Programs\SubSource\ffsubsync-venv\Scripts` یا `PATH` |

اگر ابزاری در این مسیرها نباشد، مسیر آن را در صفحه تنظیمات برنامه به‌صورت دستی وارد کنید.
نبود هر ابزار فقط قابلیت مربوط به آن را غیرفعال می‌کند؛ برنامه باید همچنان باز شود.

## فضای دیسک و پاک‌سازی

بیشتر حجم پروژه مربوط به خروجی‌های build است، نه سورس‌کد:

| مسیر | حجم تقریبی | قابل حذف؟ | نحوه بازسازی |
| --- | --- | --- | --- |
| `src-tauri/target/` | ~11.3 GB (debug ~9.3 + release ~1.8) | ✅ بله | `npm run tauri build` یا `cargo build` |
| `node_modules/` | ~162 MB | ✅ بله | `npm ci` |
| `dist/` | ~0.4 MB | ✅ بله | `npm run build` |
| `.git/` | ~1.8 MB | ❌ نه | تاریخچه ریپو |
| `src/` + `src-tauri/src/` | ~1 MB | ❌ نه | سورس‌کد |

پاک‌سازی امن (وقتی هیچ build یا برنامه‌ای در حال اجرا نیست):

```bat
rmdir /s /q src-tauri\target
rmdir /s /q dist
rmdir /s /q node_modules
```

یا برای target: `cargo clean --manifest-path src-tauri\Cargo.toml`.
هیچ‌کدام از این مسیرها در Git ردیابی نمی‌شوند (`.gitignore`). فایل‌های سورس،
`src-tauri/icons/`، `Cargo.lock` و `package-lock.json` را حذف نکنید.

## عیب‌یابی: صفحه «can't reach this page» / پروکسی

اگر برنامه صفحه‌ای شبیه «Hmmm… can't reach this page»، «localhost refused to connect» یا
«Checking the proxy and the firewall» نشان داد:

- این پیام خطای WebView2 است، نه خطای پروکسی. تنظیمات پروکسی ویندوز را تغییر ندهید.
- یعنی فایل اجرایی نسخه **توسعه** است یا با `cargo build --release` (بدون `npm run tauri build`)
  ساخته شده و به `http://localhost:5173` اشاره می‌کند.
- راه‌حل: نسخه نصب‌شده را از Start Menu اجرا کنید، یا با `npm run tauri build` دوباره installer
  بسازید. برای توسعه هم از `npm run tauri dev` استفاده کنید تا Vite بالا باشد.

## معماری

```
src-tauri/src/
├── main.rs            # نقطه ورود
├── lib.rs             # راه‌اندازی Tauri، State، بررسی ابزارها در شروع
├── models/            # ساختارهای داده مشترک (serde camelCase)
├── scanner/           # اسکن بازگشتی + naming (تشخیص S/E) + تست‌ها
├── settings/          # بارگذاری/ذخیره JSON با اعتبارسنجی
├── tools/             # تشخیص و پروب ابزارهای خارجی
├── logging/           # لاگ در حافظه + فایل + رخداد به UI
├── commands/          # دستورات Tauri (scan/settings/tools/logs)
└── state.rs           # وضعیت سراسری

src/                   # فرانت‌اند React (RTL، فارسی، Dark)
├── components/        # Sidebar، UI primitives
├── pages/             # Dashboard، Library، Processing، Logs، Settings
├── hooks/             # useAppData — State و رخدادهای Backend
├── lib/               # api.ts (invoke) و format.ts (اعداد/حجم فارسی)
└── types/             # قرینه مدل‌های Rust
```

اصل طراحی: **منطق عملیات در Rust** است؛ فرانت‌اند فقط رابط است. اجرای ابزارهای خارجی با
`std::process::Command` و آرگومان‌های جداگانه انجام می‌شود (بدون ساخت رشته شل). دسترسی‌های
Tauri حداقلی است (`core:default` + `dialog:default`).

## اصول داده‌ای (برای همه فازها)

- ویدیوی اصلی هرگز تغییر نمی‌کند
- زیرنویس‌های اولیه حفظ و هرگز بازنویسی نمی‌شوند
- هر خروجی اول Temp، بعد اعتبارسنجی، بعد انتقال امن
- پردازش گروهی فقط با کلیک صریح کاربر شروع می‌شود

## CI

GitHub Actions روی ویندوز: `npm ci`، build فرانت‌اند، `cargo fmt --check`، `clippy -D warnings`،
`cargo test` و build دودی — ببینید `.github/workflows/ci.yml`.
