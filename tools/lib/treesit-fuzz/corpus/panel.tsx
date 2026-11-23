// A small React panel: the TSX sample tools/treesit-fuzz edits.
import React, { useEffect, useMemo, useState } from "react";

interface Item {
  id: number;
  title: string;
  done: boolean;
}

type Props = {
  heading: string;
  items: Item[];
  onToggle?: (id: number) => void;
};

const Badge: React.FC<{ count: number }> = ({ count }) =>
  count > 0 ? <span className="badge">{count}</span> : null;

export function Panel({ heading, items, onToggle }: Props): JSX.Element {
  const [filter, setFilter] = useState<"all" | "open" | "done">("all");
  const [query, setQuery] = useState("");

  const shown = useMemo(
    () =>
      items
        .filter((item) => (filter === "all" ? true : filter === "done" ? item.done : !item.done))
        .filter((item) => item.title.toLowerCase().includes(query.toLowerCase())),
    [items, filter, query],
  );

  useEffect(() => {
    document.title = `${heading} (${shown.length})`;
  }, [heading, shown.length]);

  return (
    <section className="panel" aria-label={heading}>
      <header>
        <h2>
          {heading} <Badge count={items.filter((i) => !i.done).length} />
        </h2>
        <input value={query} placeholder="Search" onChange={(e) => setQuery(e.target.value)} />
        {(["all", "open", "done"] as const).map((name) => (
          <button key={name} disabled={filter === name} onClick={() => setFilter(name)}>
            {name}
          </button>
        ))}
      </header>
      {shown.length === 0 ? (
        <p className="empty">Nothing matches &quot;{query}&quot;.</p>
      ) : (
        <ul>
          {shown.map(({ id, title, done }) => (
            <li key={id} className={done ? "done" : undefined}>
              <label>
                <input type="checkbox" checked={done} onChange={() => onToggle?.(id)} />
                {title}
              </label>
            </li>
          ))}
        </ul>
      )}
      <footer>
        <>
          {shown.length} of {items.length} shown
        </>
      </footer>
    </section>
  );
}

export default Panel;
