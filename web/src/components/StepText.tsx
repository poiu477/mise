import { tidyStep } from "../recipe";

/**
 * Step text as published: "•" bullets become a list, and sentences that lost their space when the source
 * joined paragraphs ("…tender.Drain") get it back.
 */
export function StepText({ text }: { text: string }) {
  const bullets = text.split("•").map(tidyStep).filter(Boolean);
  if (bullets.length > 1) {
    return (
      <ul className="bullets">
        {bullets.map((b, i) => (
          <li key={i}>{b}</li>
        ))}
      </ul>
    );
  }
  return <p>{tidyStep(text.replace("•", ""))}</p>;
}
