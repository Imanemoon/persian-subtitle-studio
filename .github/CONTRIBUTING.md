# مشارکت در SubSync Studio / Contributing

از علاقه‌تان به مشارکت خوشحال می‌شویم! 🎉 / Thanks for wanting to contribute!

## شروع سریع / Quick start

```bash
git clone https://github.com/Imanemoon/persian-subtitle-studio.git
cd persian-subtitle-studio
npm install
npm run tauri dev        # اجرای توسعه
cd src-tauri && cargo test   # تست‌ها (بدون نیاز به شبکه/کلید)
```

پیش‌نیازها: Node 20+، Rust stable (MSVC) — بقیه (NSIS، فیک‌ها) خودکار است.

## قواعد پروژه / Ground rules

1. **هیچ پیشرفت یا موفقیتی ساختگی نیست** — هر عدد و وضعیت در UI باید از دادهٔ واقعی
   بک‌اند بیاید. این قرارداد صریح پروژه است (همین قاعده در تست‌ها هم اعمال شده).
2. **fail-closed** — در شکست، ورودی کاربر هرگز بازنویسی نمی‌شود؛ فایل موقت پاک و دلیل
   صادقانه گزارش می‌شود.
3. **هیچ secret ای در کد/تاریخچه** — کلید API فقط در Credential Manager ویندوز.
4. خروجی هر تغییر: `cargo fmt` + `cargo clippy --all-targets` (صفر warning) +
   `cargo test` + `npm run build` — CI همین‌ها را با `-D warnings` چک می‌کند.
5. تست‌های زندهٔ `#[ignore]` نیازمند ابزارهای واقعی‌اند و فقط دستی اجرا می‌شوند.

## ارسال تغییر / Submitting

- برای هر تغییر معنادار یک Issue بزن (قالب Bug report / Feature request).
- از branch جدا استفاده کن (`feat/...` یا `fix/...`) و PR به `main`.
- پیام commit: conventional style (`feat(scope): ...` / `fix(scope): ...`) — فارسی یا
  انگلیسی، فقط یکدست.
- PR باید CI سبز داشته باشد؛ توضیح «چرا» مهم‌تر از «چه» است.

## گزارش باگ مؤثر

لاگ برنامه را پیوست کن: «لاگ‌ها → کپی» یا `%LOCALAPPDATA%\com.subsync.studio\logs\` —
بدون نام فایل‌های خصوصی‌ات. نسخهٔ ویندوز و نسخهٔ Release را هم بنویس.
