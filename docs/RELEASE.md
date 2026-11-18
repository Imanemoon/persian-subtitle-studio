# راهنمای ریلیز متن‌باز — SubSync Studio

این سند، گام‌های آماده‌سازی ریلیز عمومی در گیت‌هاب را جمع می‌کند: لایسنس،
امضای دیجیتال، پیش‌نیازهای خودکار و نکات حقوقی باندل‌ها.

## ۱) لایسنس — انتخاب شما

فایل `LICENSE` هنوز **عمداً commit نشده** — انتخابش با شماست:

| گزینه | چه معنایی دارد |
|---|---|
| **MIT** | بازترین؛ هرکس استفاده، تغییر و حتی تجاری می‌کند. متنی کوتاه. |
| **GPL-3.0** | هر کسی که کد را برد و تغییر داد، باید خودش هم متن‌باز منتشر کند. |

نکتهٔ ffmpeg: برنامه ffmpeg را **در زمان اجرا دانلود می‌کند** (باندل نمی‌کند)،
پس اجبار GPL برای کل پروژه وجود ندارد — MIT هم آزادانه ممکن است.

```
# پس از انتخاب، مثلاً MIT:
# محتوای https://opensource.org/license/mit را در LICENSE بگذارید و commit بزنید.
```

## ۲) امضای دیجیتال — حقیقت ماجرا و سه مسیر

بدون امضا، Windows SmartScreen هنگام اجرای نصاب به کاربر هشدار می‌دهد.
«رفع» کامل این هشدار فقط با گواهی از یک Certificate Authority ممکن است —
هیچ راه رایگانی برای اعتماد عمومی وجود ندارد. سه مسیر واقعی:

| مسیر | هزینه | نتیجه |
|---|---|---|
| **Azure Trusted Signing** | ~$9.99/ماه | ارزان‌ترین اعتماد عمومی؛ امضا در سرویس ابری مایکروسافت. برای ریلیز متن‌باز فردی، پیشنهاد اول. |
| گواهی OV/EV (Sectigo/DigiCert و غیره) | ~$100–400/سال | فایل گواهی می‌گیری؛ EV بلافاصله اعتماد می‌گیرد، OV با انبوه دانلود. |
| بدون امضا | رایگان | هشدار SmartScreen: کاربر «More info → Run anyway» می‌زند. اعتبار با انباشت دانلود کم‌کم بالا می‌رود. |

### امضا با اسکریپت موجود

گواهی را در env ست کنید و اجرا کنید (بعد از `npm run tauri build`):

```powershell
# thumbprint گواهی نصب‌شده در مخزن ویندوز:
$env:SIGN_CERT_THUMBPRINT = "<40-hex>"
.\scripts\sign-release.ps1

# یا فایل PFX:
$env:SIGN_PFX = "C:\certs\key.pfx"
$env:SIGN_PFX_PASSWORD = "..."
.\scripts\sign-release.ps1
```

اسکریپت هر دو خروجی (`persian-subtitle-studio.exe` و نصاب NSIS) را امضا و
تأیید می‌کند و timestamp عمومی می‌زند. **هیچ secret ای در repo نیست.**

### تست با self-signed (فقط برای تست محلی — کاربران اعتماد نمی‌گیرند)

```powershell
New-SelfSignedCertificate -Type CodeSigningCert -Subject "CN=SubSync Studio Test" -CertStoreLocation Cert:\CurrentUser\My
# thumbprint خروجی را در SIGN_CERT_THUMBPRINT بگذارید؛ برای import اعتماد در «Trusted People»:
Import-Certificate -FilePath <...cer> -CertStoreLocation Cert:\CurrentUser\TrustedPeople
```

## ۳) پیش‌نیازهای خودکار — چطور کار می‌کند

نصاب NSIS عمداً کوچک می‌ماند؛ بار اول که برنامه باز شود، صفحهٔ
«راه‌اندازی» می‌آید و با یک کلیک این‌ها دانلود و نصب می‌شوند:

| ابزار | منبع | نکته |
|---|---|---|
| ffmpeg + ffprobe | ریلیز تگ‌دار GitHub گyan.dev — نسخهٔ پین‌شدهٔ 8.0 | SHA256 پین‌شده در `src-tauri/src/setup/mod.rs` (FFMPEG_SHA256) |
| Python 3.11.9 + ffsubsync 0.5.1 | python.org رسمی + pip | نصب per-user بدون admin در پوشهٔ خود برنامه |
| 7-Zip | نصاب رسمی **باندل‌شده** در نصاب | نصب با UAC به Program Files |
| sub-rs | **باندل‌شده** در نصاب | باینری خود پروژه؛ پیش‌آماده می‌شود و وارد معاونت نمی‌شود |

نسخه‌ها و hashها در `src-tauri/src/setup/mod.rs` پین‌شده‌اند — برای ارتقا،
فایل جدید را دانلود، hash بگیرید و دو const را عوض کنید.

**نکتهٔ حقوقی باندل‌ها:** ffmpeg دانلود-in-time است و توزیع نمی‌شود؛ نصاب
7-Zip redistribute رسمی (LGPL) است؛ `sub-rs.exe` باینری خودِ این پروژه است —
اگر منبع آن ریپوی دیگری است، ریپوی آن را هم متن‌باز کنید یا منبع دانلود
مطمئن معرفی کنید (نوشتنش در همین سند + tauri.conf.json resources).

## ۴) ریلیز در گیت‌هاب — چک‌لیست

1. `npm run tauri build` → نصاب در `src-tauri/target/release/bundle/nsis/`
2. امضا با `scripts/sign-release.ps1`
3. یک Release در گیت‌هاب بسازید و نصاب را به‌عنوان artifact پیوست کنید
4. در README: بج Download، اسکرین‌شات، پیش‌نیاز (Windows 10+)، توضیح صفحهٔ راه‌اندازی اول
5. اگر لایسنس را انتخاب کردید → فایل LICENSE + بخش License در README
6. CI موجود فقط unit+integration می‌راند؛ برای build خودکار ریلیز می‌توانید
   بعداً GitHub Action اضافه کنید (windows-latest + `npm run tauri build`)

## ۵) محرمانگی و امنیت

- کلید API SubSource فقط در Credential Manager ویندوز ذخیره می‌شود — هرگز
  در settings.json یا کد نیست.
- دانلودها با SHA256 پین‌شده تأیید می‌شوند؛ نصاب‌های باندل‌شده همیشه از
  resources خود نصاب می‌آیند نه از اینترنت.
- استثنای Defender فقط با کلیک صریح کاربر و پنجرهٔ UAC انجام می‌شود —
  هیچ‌چیز پشت سر کاربر elevated اجرا نمی‌شود.
