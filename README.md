<div align="center">

# 🎬 SubSync Studio

**زیرنویس فارسی، بدون دردسر — استخراج، دانلود و همگام‌سازی خودکار زیرنویس فارسی برای سریال‌ها و فیلم‌های شما**

*Automatic Persian subtitle extraction, downloading and syncing for your movies & series — on Windows, with zero manual work.*

[![Release](https://img.shields.io/github/v/release/Imanemoon/persian-subtitle-studio?label=%D8%AF%D8%A7%D9%86%D9%84%D9%88%D8%AF&color=10b981)](https://github.com/Imanemoon/persian-subtitle-studio/releases/latest)
[![CI](https://github.com/Imanemoon/persian-subtitle-studio/actions/workflows/ci.yml/badge.svg)](https://github.com/Imanemoon/persian-subtitle-studio/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%2B-lightgrey)](#نصب)

</div>

---

SubSync Studio یک برنامهٔ دسکتاپ ویندوزی است که کل چرخهٔ زیرنویس فارسی را خودکار می‌کند:
زیرنویس داخلی ویدیو را استخراج می‌کند، اگر نبود از **SubSource** دانلود می‌کند، آرشیوهای
زیرنویس را با قسمت تطبیق داده و استخراج می‌کند، و در نهایت زیرنویس را با **FFsubsync** روی
صدای ویدیو **همگام‌سازی** و کنار فیلم جانمایی می‌کند — همه‌چیز با چند کلیک و با جایگزینی
مدرن و گرافیکی برای خط لولهٔ دستی PowerShell + FFmpeg + SubSource + 7-Zip + FFsubsync.

> **English:** A Windows desktop app that automates the whole Persian-subtitle workflow:
> extract embedded tracks, download from SubSource, match & extract subtitle archives,
> sync timing with FFsubsync, and place the final `<name>.fa.synced.srt` next to your videos.

| راه‌اندازی اولین اجرا (پیش‌نیازها خودکار نصب می‌شوند) | نتیجهٔ اجرا |
| :---: | :---: |
| ![معاونت راه‌اندازی](docs/img/setup.png) | ![pipeline](docs/img/pipeline.png) |

![صفحهٔ خانه](docs/img/home.png)

## ✨ قابلیت‌ها

### معاونت راه‌اندازی خودکار
- ffmpeg، Python و FFsubsync با یک کلیک دانلود و نصب می‌شوند — per-user، بدون admin،
  با **تأیید SHA256 پین‌شده** و progress واقعی؛ هیچ نصب دستی لازم نیست
- 7-Zip و کلاینت SubSource داخل نصاب باندل شده‌اند
- افزودن یک‌کلیکی پوشه‌ها به استثناهای Windows Defender (شتاب واقعی روی هارد) و
  ساخت میان‌بر «Run as administrator»

### استخراج زیرنویس داخلی
- تحلیل با FFprobe: کدک، زبان، عنوان و Disposition هر Subtitle Stream
- تشخیص فارسی دو لایه: اول tag/عنوان Track (fa/fas/per/farsi/persian/فارسی)، بعد
  **تأیید محتوای واقعی** با رنج‌های یونیکد فارسی — tag فارسی با متن لاتین حساب نمی‌شود
- چند Track فارسی → انتخاب صریح کاربر؛ Track بدون برچسب → بررسی آزمایشی محتوا؛
  زیرنویس تصویری (PGS/VobSub/DVB) → پیام صریح نیاز به OCR
- استخراج امن: فایل موقت → اعتبارسنجی (ساختار cue + شمارش فارسی) → انتقال اتمیک به
  `<base>.fa.srt` — **بدون هیچ بازنویسی**
- زیرنویس‌های موجود skip می‌شوند (با دلیل)، لغو، فرایند FFmpeg را می‌کشد و فایل موقتش را پاک می‌کند

### دانلود از SubSource و آرشیوها
- کلید API فقط در **Credential Manager ویندوز** — هرگز در فایل تنظیمات یا کد
- Staging امن با Hard Link، بازبینی تازهٔ فایل‌سیستم، کدهای skip تایپ‌شده
- پلن آرشیو دو مرحله‌ای: بازرسی امن همهٔ اعضا → استخراج → تطبیق اپیزود → جانمایی؛
  تعارض = هر دو فایل حفظ می‌شوند

### همگام‌سازی با ffsubsync
- فراخوانی multi-segment با skip-intro-outro و محافظت کیفیت پایین (`--skip-sync-on-low-quality`)
- کد خروج 0 همیشه «موفق» نیست — طبقه‌بندی fail-closed روی marker آفست؛ آفست اعتبارسنجی‌شده در UI نمایش داده می‌شود
- workspace موقت مقیدشده (بدون فرار از ریشه، بدون بازنویسی ورودی‌ها)

### pipeline یک‌پارچه با شفافیت کامل
- ۱۲ مرحله با موانع (barrier) — هرجا ابهام باشد، برنامه می‌ایستد و با دیالوگ از شما می‌پرسد
- هر پیشرفت، شمارش و نتیجه از دادهٔ واقعی بک‌اند می‌آید — **هیچ درصد یا موفقیتی ساختگی نیست**
- لغو امن در هر لحظه، بازیابی کامل وضعیت پس از بستن/reload برنامه، تعارض‌ها و خطاها
  با دلیل صادقانه

## 📥 نصب

۱. آخرین نصاب را از [**Releases**](https://github.com/Imanemoon/persian-subtitle-studio/releases/latest)
   دانلود کنید (`SubSync Studio_x.y.z_x64-setup.exe`)
۲. نصب کنید — نصاب کوچک است؛ **بار اول که برنامه باز شود، صفحهٔ راه‌اندازی همهٔ
   پیش‌نیازها را خودش دانلود و نصب می‌کند**
۳. (پیشنهادی) در همان صفحه، پوشهٔ رسانه‌های‌تان را به استثناهای Defender اضافه کنید —
   روی هارد دیسک مکانیکی چند برابر سریع‌تر می‌شود
۴. برای دانلود از SubSource، کلید API را در تنظیمات ذخیره کنید (در Credential Manager می‌ماند)

**پیش‌نیاز سیستم:** Windows 10/11 (64-bit) — اگر WebView2 نصب نباشد، نصاب آن را دانلود می‌کند.

<details>
<summary><b>ترتیب پیدا شدن ابزارها</b> (برای کاربران حرفه‌ای)</summary>

1. مسیرهای ست‌شده در «تنظیمات → ابزارهای خارجی»
2. مسیرهای نصب‌شده توسط معاونت راه‌اندازی (`%LOCALAPPDATA%\com.subsync.studio\tools`)
3. PATH سیستم و مسیرهای استاندارد (`Program Files\7-Zip`، `%LOCALAPPDATA%\Programs\SubSource`)

</details>

## 🛠 ساخت از سورس

پیش‌نیاز: Node 20+، Rust stable (MSVC) — NSIS به‌صورت خودکار توسط tauri-cli دانلود می‌شود.

```bash
npm install
npm run tauri dev     # اجرای توسعه
npm run tauri build   # نصاب ریلیز → src-tauri/target/release/bundle/nsis/
```

تست‌ها:

```bash
cd src-tauri
cargo test                     # unit + integration (بدون نیاز به شبکه/کلید)
cargo test -- --ignored       # تست‌های زنده (ابزارهای واقعی لازم دارند)
cargo clippy --all-targets    # صفر warning هدف پروژه است
```

## 🧱 تکنولوژی

- **Backend:** Rust + Tauri 2 — هسته‌های مستقل از Tauri با callback تزریق‌پذیر، admission
  اتمیک عملیات (RAII)، workspace موقت مقیدشده، fail-closed در همهٔ مرزها
- **Frontend:** React 19 + TypeScript + Tailwind CSS v4 — RTL کامل با فونت وزیرمتن؛
  همهٔ اعداد و وضعیت‌ها فقط از بک‌اند
- **امنیت:** دانلودها با SHA256 پین‌شده تأیید می‌شوند، کلید API فقط در Credential Manager،
  هیچ عمل elevated بدون رضایت صریح کاربر اجرا نمی‌شود

## 📦 اجزای باندل‌شده

| جزء | منبع |
|---|---|
| `sub-rs.exe` (کلاینت دانلود SubSource) | باینری این پروژه |
| `7z2603-x64.exe` | نصاب رسمی [7-Zip](https://www.7-zip.org/) (LGPL) |
| `ffmpeg` / `python` / `ffsubsync` | **باندل نمی‌شوند** — در زمان اجرا از منابع رسمی با نسخهٔ پین‌شده و SHA256 دانلود می‌شوند |

## 🗺 نقشهٔ راه

- [x] فاز ۱ — اسکن واقعی پوشه‌ها، تشخیص اپیزود، داشبورد، تشخیص ابزارها، تنظیمات
- [x] فاز ۲ — موتور زیرنویس: FFprobe، استخراج، تشخیص محتوای فارسی
- [x] فاز ۳ — دانلود SubSource، آرشیوها، سینک ffsubsync، pipeline انتها-به-انتها
- [x] فاز ۴ — نصاب ویندوز + معاونت راه‌اندازی خودکار پیش‌نیازها
- [ ] OCR برای زیرنویس تصویری (PGS/VobSub)
- [ ] ویرایشگر زیرنویس و زمان‌بندی دستی

## 📄 لایسنس

[MIT](LICENSE) — ffmpeg به‌صورت runtime دانلود می‌شود، نه باندل.

## 🇬🇧 Summary (English)

SubSync Studio automates Persian subtitles for your video library: extract embedded tracks,
download from SubSource (API key stored only in Windows Credential Manager), match subtitle
archives to episodes, sync timing with ffsubsync, and place `<name>.fa.synced.srt` next to
each video — with a first-run wizard that auto-installs all prerequisites (version-pinned,
SHA256-verified, per-user), honest progress everywhere, and safe cancellation/recovery.
Built with Tauri 2 + Rust + React. Requires Windows 10+.
