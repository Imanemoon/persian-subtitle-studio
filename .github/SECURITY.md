# Security Policy / سیاست امنیتی

## نسخه‌های پشتیبانی‌شده / Supported versions

| نسخه | پشتیبانی |
|---|---|
| v0.1.x (آخرین Release) | ✅ |
| نسخه‌های قدیمی‌تر | ❌ — به آخرین Release ارتقا دهید |

## گزارش آسیب‌پذیری / Reporting a vulnerability

**از Issue عمومی برای آسیب‌پذیری‌ها استفاده نکنید.**

- ایمیل: `imunemun@gmail.com` — موضوع: `[SECURITY] SubSync Studio`
- حداکثر پاسخ: ۷ روز؛ پس از هماهنگی، فیکس در Release بعدی منتشر و اعتبار شما
  در CHANGELOG ثبت می‌شود.

## دامنه‌های حیاتی که در طراحی مراقبت شده‌اند / Security-relevant design notes

- **کلید API SubSource** فقط در Windows Credential Manager ذخیره می‌شود — هرگز در
  فایل تنظیمات، لاگ یا کد.
- **دانلودهای خودکار** (ffmpeg/python) با SHA256 پین‌شده در
  `src-tauri/src/setup/mod.rs` تأیید می‌شوند؛ هر عدم تطابق = دور انداختن فایل.
- **استخراج آرشیو** در workspace مقیدشده انجام می‌شود (بدون symlink/reparse، بدون
  فرار از ریشه، سقف تعداد/حجم).
- **عملیات elevated** (استثنای Defender، نصب 7-Zip) فقط با کلیک صریح کاربر و
  پنجرهٔ UAC اجرا می‌شود.
- فایل دانلودی و فایل موقت هرگز روی فایل اصلی کاربر بازنویسی نمی‌شوند (no-overwrite).
