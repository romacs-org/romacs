// A small task queue, as a JavaScript sample for the face oracle.
import { readFile } from 'node:fs/promises';
import * as path from 'node:path';

const DEFAULT_LIMIT = 8;
const PATTERN = /^task-(\d+)\.json$/i;

/**
 * A queue that runs at most `limit` tasks at once.
 * @param {number} limit The number of tasks in flight.
 */
export class TaskQueue {
  #running = 0;
  #pending = [];

  constructor(limit = DEFAULT_LIMIT) {
    this.limit = limit;
    this.done = new Map();
  }

  get idle() {
    return this.#running === 0 && this.#pending.length === 0;
  }

  push(task) {
    return new Promise((resolve, reject) => {
      this.#pending.push({ task, resolve, reject });
      this.#next();
    });
  }

  async #next() {
    if (this.#running >= this.limit) return;
    const entry = this.#pending.shift();
    if (!entry) return;
    this.#running += 1;
    try {
      const value = await entry.task();
      this.done.set(entry.task.name ?? `anonymous-${this.done.size}`, value);
      entry.resolve(value);
    } catch (error) {
      entry.reject(new Error(`task failed: ${error.message}`));
    } finally {
      this.#running -= 1;
      this.#next();
    }
  }
}

export async function loadTasks(dir, names) {
  const queue = new TaskQueue(4);
  const results = await Promise.all(
    names
      .filter((name) => PATTERN.test(name))
      .map((name) => queue.push(async function load() {
        const text = await readFile(path.join(dir, name), 'utf8');
        const { id = 0, weight = 1.5, ...rest } = JSON.parse(text);
        return { id: Number(id) * 2, weight, rest };
      })),
  );
  for (const [i, result] of results.entries()) {
    if (typeof result !== 'object' || result === null) {
      console.warn('bad task', i);
    }
  }
  return results.length > 0 ? results : null;
}
