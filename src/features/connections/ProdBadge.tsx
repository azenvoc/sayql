import type { ConnectionConfig } from "../../api";
import { Badge } from "../../components/ui";

/** Whether the person tagged the connection as production (connections saved before 0.4.8 aren't). */
export const isProduction = (c: Pick<ConnectionConfig, "production"> | null | undefined) =>
  !!c?.production;

/** "PROD", wherever a production connection shows: the navigator, tabs, the top bar and the status bar. */
export function ProdBadge({
  connection,
}: {
  connection: Pick<ConnectionConfig, "production"> | null | undefined;
}) {
  if (!isProduction(connection)) return null;
  return (
    <Badge tone="danger" title="Tagged as production in this connection's settings">
      PROD
    </Badge>
  );
}
