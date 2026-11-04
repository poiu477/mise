// Weeks run Monday to Sunday and are named by their Monday, as the API stores them. Dates are the viewer's local days.

const iso = (d: Date) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
const parse = (week: string) => {
  const [y, m, d] = week.split("-").map(Number);
  return new Date(y, m - 1, d);
};

export function mondayOf(day: Date = new Date()): string {
  const d = new Date(day.getFullYear(), day.getMonth(), day.getDate());
  d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
  return iso(d);
}

/** The Monday of the week holding `day` ("2026-11-05" → "2026-11-02"). */
export const weekOf = (day: string) => mondayOf(parse(day));

export function addWeeks(week: string, n: number): string {
  const d = parse(week);
  d.setDate(d.getDate() + n * 7);
  return iso(d);
}

/** "This week", "Next week", "Last week", or the dates: "Nov 9 – 15". */
export function weekLabel(week: string, today: Date = new Date()): string {
  const now = mondayOf(today);
  if (week === now) return "This week";
  if (week === addWeeks(now, 1)) return "Next week";
  if (week === addWeeks(now, -1)) return "Last week";
  const start = parse(week);
  const end = parse(addWeeks(week, 1));
  end.setDate(end.getDate() - 1);
  const month = (d: Date) => d.toLocaleDateString("en-US", { month: "short" });
  const tail = month(start) === month(end) ? `${end.getDate()}` : `${month(end)} ${end.getDate()}`;
  return `${month(start)} ${start.getDate()} – ${tail}`;
}
