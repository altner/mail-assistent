import type { Address } from "./types";

const time = new Intl.DateTimeFormat("de-DE", { hour: "2-digit", minute: "2-digit" });
const day = new Intl.DateTimeFormat("de-DE", { day: "2-digit", month: "2-digit", year: "numeric" });
const full = new Intl.DateTimeFormat("de-DE", { dateStyle: "full", timeStyle: "short" });

/** Kurzform für Listen: heute nur Uhrzeit, sonst Datum. */
export function shortDate(iso: string | null): string {
  if (!iso) return "–";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const now = new Date();
  return d.toDateString() === now.toDateString() ? time.format(d) : day.format(d);
}

export function longDate(iso: string | null): string {
  if (!iso) return "unbekannt";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : full.format(d);
}

export function sender(name: string | null, addr: string | null): string {
  return name || addr || "(unbekannt)";
}

/** Empfänger aus dem gespeicherten JSON-Array als lesbare Liste. */
export function recipients(json: string | null): string {
  if (!json) return "–";
  try {
    return (JSON.parse(json) as Address[])
      .map((a) => (a.name ? `${a.name} <${a.address}>` : a.address))
      .join(", ");
  } catch {
    return json;
  }
}
