import { Search } from "lucide-react";
import { EmptyState, Logo, TextInput } from "../../components/ui";
import { filterChoices, type DatabaseChoice } from "./kinds";
import styles from "./DatabasePicker.module.css";

/**
 * The first page of New connection: a card with its logo for every database SayQL connects to,
 * with a filter. Enter picks the only card left.
 */
export function DatabasePicker({
  filter,
  onFilter,
  onPick,
}: {
  filter: string;
  onFilter: (filter: string) => void;
  onPick: (choice: DatabaseChoice) => void;
}) {
  const choices = filterChoices(filter);
  return (
    <section className={styles.picker} aria-label="Select your database">
      <div className={styles.filter}>
        <Search size={14} className={styles.filterIcon} aria-hidden="true" />
        <TextInput
          autoFocus
          aria-label="Filter databases"
          placeholder="Type part of a database name to filter"
          value={filter}
          onChange={(e) => onFilter(e.target.value)}
          onKeyDown={(e) => {
            const [only, ...rest] = choices;
            if (e.key === "Enter" && only && rest.length === 0) {
              e.preventDefault();
              onPick(only);
            }
          }}
        />
      </div>
      {choices.length === 0 ? (
        <EmptyState icon={Search} title="No database matches">
          SayQL connects to PostgreSQL, MySQL, MariaDB, SQL Server, Azure SQL and SQLite.
        </EmptyState>
      ) : (
        <ul className={styles.grid}>
          {choices.map((c) => (
            <li key={c.id}>
              <button
                type="button"
                className={styles.card}
                onClick={() => onPick(c)}
                data-testid={`database-${c.id}`}
              >
                <Logo id={c.logo} size={40} />
                <span className={styles.name}>{c.name}</span>
                <span className={styles.detail}>{c.detail}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
