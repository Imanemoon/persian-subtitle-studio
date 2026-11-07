export function faNum(value: number | string): string {
  return Number(value).toLocaleString("fa-IR");
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "۰";
  const units = ["بایت", "کیلوبایت", "مگابایت", "گیگابایت", "ترابایت"];
  let value = bytes;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024;
    i += 1;
  }
  const rounded = i === 0 || value >= 100 ? Math.round(value) : Math.round(value * 10) / 10;
  return `${faNum(rounded)} ${units[i]}`;
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${faNum(ms)} میلی‌ثانیه`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${faNum(Math.round(seconds * 10) / 10)} ثانیه`;
  const minutes = Math.floor(seconds / 60);
  const rest = Math.round(seconds % 60);
  return `${faNum(minutes)} دقیقه و ${faNum(rest)} ثانیه`;
}

export function formatTimestamp(ms: number | null): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString("fa-IR", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}
