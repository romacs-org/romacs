// A small linked list with a cursor: the JavaScript sample tools/treesit-fuzz edits.
"use strict";

const EMPTY = Symbol("empty");

class Node {
  constructor(value, next = null) {
    this.value = value;
    this.next = next;
  }
}

export class List {
  #head = null;
  #size = 0;

  static from(iterable) {
    const list = new List();
    for (const value of [...iterable].reverse()) {
      list.push(value);
    }
    return list;
  }

  get size() {
    return this.#size;
  }

  push(value) {
    this.#head = new Node(value, this.#head);
    this.#size += 1;
    return this;
  }

  pop() {
    if (this.#head === null) return EMPTY;
    const { value, next } = this.#head;
    this.#head = next;
    this.#size -= 1;
    return value;
  }

  *[Symbol.iterator]() {
    for (let node = this.#head; node; node = node.next) {
      yield node.value;
    }
  }

  map(fn) {
    return List.from([...this].map((value, index) => fn(value, index)));
  }

  filter(predicate) {
    return List.from([...this].filter(predicate));
  }

  reduce(fn, initial) {
    let acc = initial;
    for (const value of this) acc = fn(acc, value);
    return acc;
  }

  toString() {
    return `List(${[...this].join(", ")})`;
  }
}

export function chunks(list, size = 2) {
  const out = [];
  let current = [];
  for (const value of list) {
    current.push(value);
    if (current.length === size) {
      out.push(current);
      current = [];
    }
  }
  return current.length ? [...out, current] : out;
}

export async function collect(source, { limit = Infinity, signal } = {}) {
  const list = new List();
  for await (const item of source) {
    if (signal?.aborted || list.size >= limit) break;
    list.push(item);
  }
  return list;
}

const sum = (list) => list.reduce((a, b) => a + b, 0);
const describe = (list) =>
  list.size === 0 ? "empty" : list.size === 1 ? "one item" : `${list.size} items`;

export default { List, chunks, collect, sum, describe, EMPTY };
