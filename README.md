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
| ۲ | Subtitle Engine — FFprobe، استخراج زیرنویس داخلی، تشخیص محتوای فارسی | 🔜 بعدی |
| ۳ | Download & Archive — SubSource، Staging با Hard Link، ZIP/RAR با 7-Zip | ⏳ |
| ۴ | Sync Engine — FFsubsync، Workerها، Stop/Retry | ⏳ |
| ۵ | Packaging — Installer ویندوز، مستندات نهایی | ⏳ |

## امکانات فعلی (فاز ۱)

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

## پیش‌نیازها

- Windows 10/11 + WebView2
- [Node.js](https://nodejs.org) ≥ 22 و npm
- [Rust](https://rustup.rs) (toolchain `stable-msvc`) + MSVC Build Tools
- ابزارهای خارجی (اختیاری بسته به فاز): FFmpeg/FFprobe در PATH، 7-Zip، sub-rs، ffs

## اجرای محیط توسعه

```bash
npm install
npm run tauri dev
```

## تست‌ها و بررسی‌ها

```bash
cd src-tauri
cargo fmt
cargo check
cargo test --lib          # تست‌های واحد
cargo test --lib -- --ignored   # تست زندهٔ تشخیص ابزارها روی همین سیستم
cargo clippy --all-targets -- -D warnings
```

build فرانت‌اند:

```bash
npm run build
```

## ساخت نسخه Release ویندوز

```bash
npm run tauri build
```

خروجی NSIS Installer در `src-tauri/target/release/bundle/` قرار می‌گیرد.

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
