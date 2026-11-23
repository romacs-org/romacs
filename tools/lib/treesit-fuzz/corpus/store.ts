// A small typed store: the TypeScript sample tools/treesit-fuzz edits.
export type Id = string & { readonly brand: unique symbol };

export interface Entry<T> {
  id: Id;
  value: T;
  tags: ReadonlyArray<string>;
  updated?: Date;
}

export enum Change {
  Added = "added",
  Removed = "removed",
  Updated = "updated",
}

type Listener<T> = (change: Change, entry: Entry<T>) => void;

export class Store<T extends object> implements Iterable<Entry<T>> {
  private readonly entries = new Map<Id, Entry<T>>();
  private listeners: Listener<T>[] = [];

  constructor(private readonly name: string, initial: Entry<T>[] = []) {
    for (const entry of initial) {
      this.entries.set(entry.id, entry);
    }
  }

  get size(): number {
    return this.entries.size;
  }

  add(id: Id, value: T, ...tags: string[]): Entry<T> {
    const entry: Entry<T> = { id, value, tags, updated: new Date() };
    this.entries.set(id, entry);
    this.emit(Change.Added, entry);
    return entry;
  }

  update(id: Id, patch: Partial<T>): Entry<T> | undefined {
    const old = this.entries.get(id);
    if (!old) return undefined;
    const entry = { ...old, value: { ...old.value, ...patch }, updated: new Date() };
    this.entries.set(id, entry);
    this.emit(Change.Updated, entry);
    return entry;
  }

  remove(id: Id): boolean {
    const entry = this.entries.get(id);
    if (entry === undefined) {
      return false;
    }
    this.entries.delete(id);
    this.emit(Change.Removed, entry);
    return true;
  }

  find(predicate: (value: T) => boolean): Entry<T>[] {
    return [...this.entries.values()].filter((e) => predicate(e.value));
  }

  tagged(tag: string): Entry<T>[] {
    return this.find(() => true).filter(({ tags }) => tags.includes(tag));
  }

  subscribe(listener: Listener<T>): () => void {
    this.listeners.push(listener);
    return () => {
      this.listeners = this.listeners.filter((l) => l !== listener);
    };
  }

  *[Symbol.iterator](): Iterator<Entry<T>> {
    yield* this.entries.values();
  }

  private emit(change: Change, entry: Entry<T>): void {
    for (const listener of this.listeners) {
      try {
        listener(change, entry);
      } catch (error: unknown) {
        console.error(`${this.name}: listener failed`, error);
      }
    }
  }
}

export async function load<T extends object>(url: string, parse: (raw: unknown) => T): Promise<Store<T>> {
  const response = await fetch(url, { headers: { Accept: "application/json" } });
  if (!response.ok) {
    throw new Error(`load ${url}: ${response.status} ${response.statusText}`);
  }
  const raw = (await response.json()) as Array<{ id: string; value: unknown; tags?: string[] }>;
  return new Store(url, raw.map(({ id, value, tags = [] }) => ({ id: id as Id, value: parse(value), tags })));
}

export const counts = <T extends object>(store: Store<T>): Record<string, number> =>
  [...store].reduce<Record<string, number>>((acc, { tags }) => {
    tags.forEach((tag) => (acc[tag] = (acc[tag] ?? 0) + 1));
    return acc;
  }, {});

namespace Format {
  export const date = (d?: Date): string => (d ? d.toISOString().slice(0, 10) : "never");
  export function entry<T>(e: Entry<T>, show: (v: T) => string = String): string {
    return `${e.id} [${e.tags.join(", ")}] ${show(e.value)} (${date(e.updated)})`;
  }
}

export default Format;
