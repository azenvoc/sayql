import { useEffect, useState } from "react";
import { commands } from "../../api";
import { Alert } from "../../components/ui";
import { errorText } from "../../lib/errors";

/** Below this many dollars, SayQL warns that the credit is running low. */
export const LOW_CREDIT_USD = 5;

/** "$12.34". */
export function dollars(usd: number): string {
  return `$${usd.toFixed(2)}`;
}

/** The warning for a low or empty balance; null when there's plenty left. */
export function lowCreditText(usd: number): string | null {
  if (usd <= 0)
    return "Your SayQL AI credit has run out, so AI requests stop until it's topped up. Top up at sayql.dev/#pricing with the email you bought it with.";
  if (usd < LOW_CREDIT_USD)
    return `Your SayQL AI credit is running low: ${dollars(usd)} left. Top up at sayql.dev/#pricing with the email you bought it with.`;
  return null;
}

/** What's left on the saved credit key, under the key field in Settings. Asks once when shown. */
export function CreditsBalance() {
  const [usd, setUsd] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    commands.creditsBalance().then(setUsd, (e) => setError(errorText(e)));
  }, []);

  if (error) return <Alert tone="warning">Couldn't check the balance: {error}</Alert>;
  if (usd == null) return null;
  const low = lowCreditText(usd);
  return (
    <Alert tone={low ? (usd <= 0 ? "danger" : "warning") : "info"}>
      {low ??
        `Balance: ${dollars(usd)} left. Each request is charged at Anthropic's API prices plus 20%.`}
    </Alert>
  );
}
