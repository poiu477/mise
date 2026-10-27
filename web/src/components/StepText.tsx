/**
 * Step text as published: "•" bullets become a list, and sentences that lost their space when the source
 * joined paragraphs ("…tender.Drain") get it back.
 */
export function StepText({ text }: { text: string }) {
  const tidy = (s: string) => s.replace(/\s+/g, " ").replace(/([a-z0-9)%])\.([A-Z][a-z])/g, "$1. $2").trim();
  const bullets = text.split("•").map(tidy).filter(Boolean);
  if (bullets.length > 1) {
    return (
      <ul className="bullets">
        {bullets.map((b, i) => (
          <li key={i}>{b}</li>
        ))}
      </ul>
    );
  }
  return <p>{tidy(text.replace("•", ""))}</p>;
}
