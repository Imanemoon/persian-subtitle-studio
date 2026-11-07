import type { ButtonHTMLAttributes, ReactNode } from "react";
import type { LucideIcon } from "lucide-react";

export function Card({
  children,
  className = "",
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={`rounded-xl border border-line bg-surface-1 ${className}`}>
      {children}
    </div>
  );
}

type ButtonVariant = "primary" | "ghost" | "danger";

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: ButtonVariant;
  loading?: boolean;
};

export function Button({
  variant = "ghost",
  loading = false,
  children,
  className = "",
  disabled,
  ...rest
}: ButtonProps) {
  const styles: Record<ButtonVariant, string> = {
    primary: "bg-accent text-zinc-950 hover:bg-accent-strong font-semibold",
    ghost: "border border-line bg-surface-2 text-ink-1 hover:bg-zinc-800",
    danger: "border border-red-500/30 bg-red-500/10 text-red-400 hover:bg-red-500/20",
  };
  return (
    <button
      className={`inline-flex h-10 items-center justify-center gap-2 rounded-lg px-4 text-sm transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${styles[variant]} ${className}`}
      disabled={disabled || loading}
      {...rest}
    >
      {loading && (
        <span className="size-4 animate-spin rounded-full border-2 border-current border-t-transparent" />
      )}
      {children}
    </button>
  );
}

type BadgeTone = "neutral" | "success" | "warn" | "danger" | "info";

export function Badge({
  tone = "neutral",
  children,
}: {
  tone?: BadgeTone;
  children: ReactNode;
}) {
  const tones: Record<BadgeTone, string> = {
    neutral: "bg-zinc-800/80 text-ink-2",
    success: "bg-emerald-500/10 text-emerald-400",
    warn: "bg-amber-500/10 text-amber-400",
    danger: "bg-red-500/10 text-red-400",
    info: "bg-sky-500/10 text-sky-400",
  };
  return (
    <span
      className={`inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-xs font-medium ${tones[tone]}`}
    >
      {children}
    </span>
  );
}

export function StatCard({
  icon: Icon,
  label,
  value,
  sub,
  tone = "neutral",
}: {
  icon: LucideIcon;
  label: string;
  value: string;
  sub?: string;
  tone?: BadgeTone;
}) {
  const iconTones: Record<BadgeTone, string> = {
    neutral: "text-ink-2 bg-zinc-800/60",
    success: "text-emerald-400 bg-emerald-500/10",
    warn: "text-amber-400 bg-amber-500/10",
    danger: "text-red-400 bg-red-500/10",
    info: "text-sky-400 bg-sky-500/10",
  };
  return (
    <Card className="p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-xs text-ink-3">{label}</div>
          <div className="mt-1 text-2xl font-bold leading-8 text-ink-1">{value}</div>
          {sub && <div className="mt-1 truncate text-xs text-ink-3">{sub}</div>}
        </div>
        <div
          className={`flex size-9 shrink-0 items-center justify-center rounded-lg ${iconTones[tone]}`}
        >
          <Icon className="size-4.5" />
        </div>
      </div>
    </Card>
  );
}

export function Toggle({
  checked,
  onChange,
  label,
  hint,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
  hint?: string;
}) {
  return (
    <button
      type="button"
      onClick={() => onChange(!checked)}
      className="flex w-full items-center justify-between gap-4 rounded-lg border border-line bg-surface-2 px-4 py-3 text-right transition-colors hover:bg-zinc-800/60"
    >
      <span>
        <span className="block text-sm text-ink-1">{label}</span>
        {hint && <span className="mt-0.5 block text-xs text-ink-3">{hint}</span>}
      </span>
      <span
        className={`relative h-6 w-11 shrink-0 rounded-full transition-colors ${
          checked ? "bg-accent" : "bg-zinc-700"
        }`}
      >
        <span
          className={`absolute top-0.5 size-5 rounded-full bg-white shadow transition-all ${
            checked ? "start-0.5" : "start-[22px]"
          }`}
        />
      </span>
    </button>
  );
}

export function ProgressBar({ active }: { active: boolean }) {
  if (!active) return null;
  return (
    <div className="h-1.5 w-full overflow-hidden rounded-full bg-zinc-800">
      <div className="h-full w-1/3 animate-progress rounded-full bg-accent" />
    </div>
  );
}

export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
}: {
  icon: LucideIcon;
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 rounded-xl border border-dashed border-line px-6 py-16 text-center">
      <div className="flex size-12 items-center justify-center rounded-full bg-zinc-800/60">
        <Icon className="size-6 text-ink-3" />
      </div>
      <div className="text-base font-semibold text-ink-1">{title}</div>
      <p className="max-w-md text-sm leading-6 text-ink-3">{description}</p>
      {action && <div className="mt-2 flex gap-2">{action}</div>}
    </div>
  );
}

export function SectionTitle({
  icon: Icon,
  title,
  extra,
}: {
  icon?: LucideIcon;
  title: string;
  extra?: ReactNode;
}) {
  return (
    <div className="mb-3 flex items-center justify-between">
      <div className="flex items-center gap-2 text-sm font-semibold text-ink-1">
        {Icon && <Icon className="size-4 text-accent" />}
        {title}
      </div>
      {extra}
    </div>
  );
}
