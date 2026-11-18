import { Component, type ErrorInfo, type ReactNode } from "react";
import { AlertTriangle, RefreshCw, RotateCcw } from "lucide-react";

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
}

/**
 * مرز خطای ریشه — هر استثنای رندر در درخت React فقط این نما را جایگزین
 * می‌کند، نه یک صفحهٔ سیاه بی‌توضیح. کاربر می‌تواند بدون از دست دادن
 * برنامه تلاش دوباره کند یا صفحه را از نو بارگذاری کند.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // لاگ فقط برای تشخیص — هیچ داده‌ای به بیرون فرستاده نمی‌شود.
    console.error("خطای رندر رابط کاربری:", error, info.componentStack);
  }

  private retry = () => {
    this.setState({ error: null });
  };

  private reload = () => {
    window.location.reload();
  };

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;

    return (
      <div
        dir="rtl"
        className="flex h-screen flex-col items-center justify-center gap-5 bg-surface-0 px-6 text-center text-ink-1"
      >
        <div className="flex size-12 items-center justify-center rounded-full bg-red-500/10">
          <AlertTriangle className="size-6 text-red-400" />
        </div>
        <div className="space-y-1.5">
          <h1 className="text-base font-bold">نمایش این صفحه با خطا متوقف شد</h1>
          <p className="max-w-md text-sm leading-6 text-ink-2">
            فایل‌های شما دست‌نخورده‌اند. می‌توانید دوباره تلاش کنید؛ اگر خطا
            تکرار شد، برنامه را از نو بارگذاری کنید.
          </p>
        </div>
        <div className="flex flex-wrap items-center justify-center gap-2">
          <button
            onClick={this.retry}
            className="inline-flex items-center gap-1.5 rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-surface-0 transition-opacity hover:opacity-90"
          >
            <RotateCcw className="size-4" />
            تلاش دوباره
          </button>
          <button
            onClick={this.reload}
            className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface-2 px-4 py-2 text-sm text-ink-1 transition-colors hover:border-ink-3"
          >
            <RefreshCw className="size-4" />
            بارگذاری مجدد
          </button>
        </div>
        <details className="max-w-lg text-start text-[11px] text-ink-3">
          <summary className="cursor-pointer text-center hover:text-ink-2">
            جزئیات فنی
          </summary>
          <pre
            dir="ltr"
            className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap break-all rounded-lg border border-line bg-surface-2 p-3 font-mono"
          >
            {error.message || String(error)}
          </pre>
        </details>
      </div>
    );
  }
}
