const MIN = 1;
const MAX = 12;

type Change = number | ((current: number) => number);

/** Serving count: a stepper, with the published sizes one tap away. */
export function Servings({ value, published, onChange }: { value: number; published: number[]; onChange: (n: Change) => void }) {
  return (
    <section className="servings" aria-label="Servings">
      <span className="option-label">Servings</span>
      <div className="stepper">
        <button type="button" onClick={() => onChange((c) => Math.max(MIN, c - 1))} disabled={value <= MIN} aria-label="Fewer servings">
          −
        </button>
        <output aria-live="polite">{value}</output>
        <button type="button" onClick={() => onChange((c) => Math.min(MAX, c + 1))} disabled={value >= MAX} aria-label="More servings">
          +
        </button>
      </div>
      {published.length > 0 && (
        <div className="published">
          <span>Published:</span>
          {published.map((n) => (
            <button key={n} type="button" aria-pressed={n === value} onClick={() => onChange(n)}>
              {n}
            </button>
          ))}
        </div>
      )}
    </section>
  );
}
