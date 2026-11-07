# مرجع: اسکریپت قبلی `Extract-PersianSubs.ps1`

مسیر اصلی روی این سیستم: `%USERPROFILE%\PowerShellTools\Extract-PersianSubs.ps1`
(این اسکریپت را تغییر نمی‌دهیم و حذف نمی‌کنیم؛ صرفاً مرجع انتقال منطق به Rust است.)

این سند رفتار تأییدشدهٔ اسکریپت (پردازش موفق ۹۸ قسمت Dexter) را خلاصه می‌کند تا
فازهای ۲ تا ۴ دقیقاً همان رفتار (با اصلاحات) پیاده کنند.

## پارامترها

- `Path` (پیش‌فرض: پوشه جاری)، `SyncWorkers` (۱..۴، پیش‌فرض ۲)، `NoDownload`، `NoSync`
- پوشه‌های مستثنا: `Ted Lasso`، `Better Call Saul`
- پسوندهای ویدیویی: `.mkv .mp4 .m4v .mov .avi .ts .m2ts .mts .webm .mpg .mpeg .vob .wmv .flv`
- پوشه‌های `_SubSource_*` هم از اسکن مستثنا هستند (پوشه کاری خود اسکریپت)

## فاز ۱ اسکریپت — استخراج زیرنویس داخلی (فاز ۲ برنامه ما)

- اگر `<base>.fa.srt` موجود باشد یا `<base>.srt` موجود محتوای فارسی داشته باشد → رد شود.
  رنج کاراکتر فارسی: `[\u0600-\u06FF\uFB50-\uFDFF\uFE70-\uFEFF]`
- ffprobe: `-v error -select_streams s -show_entries "stream=index,codec_name:stream_tags=language,language_ietf,title" -of json`
- تطبیق زبان/عنوان با: `(?i)(?<![a-z])(fa|fas|per|farsi|persian)(?![a-z])|فارسی`
- زیرنویس تصویری (`hdmv_pgs_subtitle|dvd_subtitle|dvb_subtitle|xsub`) → شمرده شود و رد شود (OCR خارج از scope)
- استخراج: `ffmpeg -nostdin -hide_banner -loglevel error -n -i <video> -map "0:<index>" -c:s srt <temp>`
  با نام موقت `<base>.fa.extracting.<guid>.srt` → اعتبارسنجی (وجود + حجم > 0) → انتقال بدون بازنویسی

## فاز ۲ اسکریپت — دانلود SubSource (فاز ۳ برنامه ما)

- پوشه Staging روی همان درایو: `<root>\_SubSource_Batch_<guid>` + Hard Link ویدیوها
  (در صورت شکست، فایل از دانلود خارج می‌شود) + Hard Link زیرنویس‌های fa موجود
  (برای `--skip-existing`)
- اجرا: `sub-rs --directory <stageRoot> --lang fa --top 1 --skip-existing`
- خروجی CLI به‌تنهایی SUCCESS حساب نمی‌شود؛ وجود فایل واقعی بررسی می‌شود
- **بازیابی ZIP/RAR**: اگر بعد از دانلود `<base>.fa.(srt|ass|ssa|vtt|sub)` نبود →
  آرشیوهای `<base>_sub*.zip` در `sub/` و آرشیوهای ذخیره‌شدهٔ `_SubSource_ZIP_Failures`
  بررسی می‌شوند. اولویت: آرشیو دقیقاً همان قسمت (بدون بازه) بعد آرشیوهای بازه‌ای مثل
  `S04E01-12`. الگوی قسمت: `(?i)(?:^|[^A-Z0-9])S0?{s}E0?{e}(?:[^0-9]|$)`
- استخراج با 7z: `7z x -y -o<dir>` — کد خروج > 1 = خطا (کد 1 فقط هشدار).
  7-Zip فرمت واقعی را تشخیص می‌دهد؛ RAR با پسوند ZIP هم باز می‌شود
- انتخاب بین چند زیرنویس: اولویت `1080p` سپس `720p` سپس حجم بزرگ‌تر
- کپی خروجی با `FileMode::CreateNew` (هرگز بازنویسی نمی‌کند)
- خطای آرشیو → کپی به `_SubSource_ZIP_Failures` برای بررسی بعدی + گزارش صریح
- انتقال نهایی فقط اگر مقصد موجود نباشد؛ حذف کامل Staging در `finally`

## فاز ۳ و ۴ اسکریپت — سینک (فاز ۴ برنامه ما)

- خروجی: `<base>.fa.synced.srt` — اگر موجود باشد → `alreadySynced`
- منبع: اولین `<base>.fa.(srt|ass|ssa|vtt|sub)` غیرخالی، وگرنه `<base>.srt` با محتوای فارسی
- اجرای موازی (RunspacePool = SyncWorkers) با:
  `ffs <video> -i <sub> -o <out> --multi-segment-sync --segment-count 6 --skip-intro-outro --skip-sync-on-low-quality --parallel-workers 1`
- اعتبارسنجی: کد خروج 0 + وجود خروجی + حجم > 0 → `Synced`؛ کد 0 بدون خروجی →
  `LowQualityOrNoOutput`؛ غیر از این → `Failed` (۵ خط آخر خروجی نگه داشته می‌شود)

## اصلاحاتی که در برنامه جدید الزامی است

1. رجکس اسکریپت `S(\d{1,2})E(\d{1,2})` ناواقعی است؛ در برنامه گارد `E01 ≠ E010` داریم
2. انتخاب خودکار زیرنویس در صورت ابهام (چند کاندید) نباید بی‌سؤال انجام شود → UI
3. `--parallel-workers 1` داخلی ffs + مدیریت Worker بیرونی در Rust
4. لغو عملیات نباید فرایند فرزند بی‌سرپرست بگذارد
