import type { ConnectionConfig, DatabaseKind } from "../../api";
import type { LogoId } from "../../components/ui";

export interface KindInfo {
  /** Full name, e.g. in the top bar tooltip. */
  label: string;
  /** Short name for the type picker. */
  option: string;
  /** Two-letter tag in the navigator. */
  short: string;
  defaultPort: number | null;
  /** Encryption on by default (required by Azure SQL). */
  defaultSsl: boolean;
  hostPlaceholder: string;
  /** CSS color token for the kind's icon. */
  color: string;
  logo: LogoId;
}

export const DATABASE_KINDS: Record<DatabaseKind, KindInfo> = {
  postgres: {
    label: "PostgreSQL",
    option: "PostgreSQL",
    short: "PG",
    defaultPort: 5432,
    defaultSsl: false,
    hostPlaceholder: "localhost",
    color: "var(--color-kind-postgres)",
    logo: "postgresql",
  },
  mysql: {
    label: "MySQL / MariaDB",
    option: "MySQL",
    short: "MY",
    defaultPort: 3306,
    defaultSsl: false,
    hostPlaceholder: "localhost",
    color: "var(--color-kind-mysql)",
    logo: "mysql",
  },
  mssql: {
    label: "SQL Server / Azure SQL",
    option: "SQL Server",
    short: "MS",
    defaultPort: 1433,
    defaultSsl: true,
    hostPlaceholder: "myserver.database.windows.net or host,port",
    color: "var(--color-kind-mssql)",
    logo: "sqlserver",
  },
  sqlite: {
    label: "SQLite",
    option: "SQLite",
    short: "SL",
    defaultPort: null,
    defaultSsl: false,
    hostPlaceholder: "",
    color: "var(--color-kind-sqlite)",
    logo: "sqlite",
  },
};

export const KIND_OPTIONS = (Object.keys(DATABASE_KINDS) as DatabaseKind[]).map((k) => ({
  value: k,
  label: DATABASE_KINDS[k].option,
}));

/** A card on the New connection page: a database people look for by name, and how it connects. */
export interface DatabaseChoice {
  id: string;
  name: string;
  logo: LogoId;
  /** What else it covers, under the name. */
  detail: string;
  /** More words the filter matches. */
  keywords: string;
  kind: DatabaseKind;
  /** Settings that differ from the kind's defaults. */
  preset?: Partial<ConnectionConfig>;
}

/** Every database SayQL connects to, in the order the New connection page shows them. */
export const DATABASE_CHOICES: DatabaseChoice[] = [
  {
    id: "postgresql",
    name: "PostgreSQL",
    logo: "postgresql",
    detail: "Also Supabase, Neon, Amazon RDS and Aurora",
    keywords: "postgres pg psql supabase neon rds aurora timescale cockroach",
    kind: "postgres",
  },
  {
    id: "mysql",
    name: "MySQL",
    logo: "mysql",
    detail: "Also PlanetScale, Amazon RDS and Aurora",
    keywords: "planetscale rds aurora tidb",
    kind: "mysql",
  },
  {
    id: "mariadb",
    name: "MariaDB",
    logo: "mariadb",
    detail: "Through the MySQL driver",
    keywords: "maria mysql",
    kind: "mysql",
  },
  {
    id: "sqlserver",
    name: "SQL Server",
    logo: "sqlserver",
    detail: "Microsoft SQL Server 2012 and later",
    keywords: "microsoft mssql tsql t-sql",
    kind: "mssql",
  },
  {
    id: "azuresql",
    name: "Azure SQL",
    logo: "azuresql",
    detail: "Azure SQL Database and Managed Instance",
    keywords: "microsoft mssql azure cloud",
    kind: "mssql",
    preset: { ssl: true, host: "" },
  },
  {
    id: "sqlite",
    name: "SQLite",
    logo: "sqlite",
    detail: "A database file on this computer",
    keywords: "file local lite",
    kind: "sqlite",
  },
];

/** The choices whose name, detail or keywords contain every word of `filter`. */
export function filterChoices(filter: string): DatabaseChoice[] {
  const words = filter.toLowerCase().split(/\s+/).filter(Boolean);
  return DATABASE_CHOICES.filter((c) => {
    const text = `${c.name} ${c.detail} ${c.keywords}`.toLowerCase();
    return words.every((w) => text.includes(w));
  });
}
