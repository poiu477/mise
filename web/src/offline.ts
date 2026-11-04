// The shopping list on this device, so it works in a shop with no signal: the last copy of each week's list, and
// the ticks and additions made since, waiting to be sent. Kept per account and cleared on signing out.
import type { Me, ShoppingList } from "./api/client";

/** A change made on this device and not yet confirmed by the server. */
export type Change =
  | { kind: "tick"; week: string; key: string; checked: boolean }
  | { kind: "tick_extra"; week: string; id: string; checked: boolean }
  /** Added by hand; `id` is this device's own ("local-…") until the server gives one. */
  | { kind: "add_extra"; week: string; id: string; name: string; checked: boolean };

export type Saved = { list: ShoppingList; savedAt: number };

/** Where the copy lives: localStorage in the browser, a map in tests. */
export interface Store {
  get(key: string): string | null;
  set(key: string, value: string): void;
  remove(key: string): void;
  keys(): string[];
}

const PREFIX = "mise:";

// Storage can be missing or refuse (private windows, full disks): the list then simply isn't kept.
export const browserStore: Store = {
  get: (k) => {
    try {
      return localStorage.getItem(k);
    } catch {
      return null;
    }
  },
  set: (k, v) => {
    try {
      localStorage.setItem(k, v);
    } catch {
      /* not kept */
    }
  },
  remove: (k) => {
    try {
      localStorage.removeItem(k);
    } catch {
      /* nothing to remove */
    }
  },
  keys: () => {
    try {
      return Object.keys(localStorage);
    } catch {
      return [];
    }
  },
};

function read<T>(store: Store, key: string): T | null {
  const raw = store.get(key);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as T;
  } catch {
    return null;
  }
}

const SIGN_IN = `${PREFIX}sign-in`;
const newSignIn = () => `${Date.now()}-${Math.random().toString(36).slice(2)}`;

/** The signed-in account, remembered so the app opens without the server. Each sign-in gets an id of its own,
 * kept while the same account stays signed in (the app checks again every time it opens) and gone on signing
 * out, so work begun in one sign-in can't touch the next. */
export function rememberMe(me: Me, store: Store = browserStore) {
  if (rememberedMe(store)?.id !== me.id || !store.get(SIGN_IN)) store.set(SIGN_IN, newSignIn());
  store.set(`${PREFIX}me`, JSON.stringify(me));
}
export const rememberedMe = (store: Store = browserStore) => read<Me>(store, `${PREFIX}me`);

/** Signing out leaves nothing of the household's on the device. */
export function forgetDevice(store: Store = browserStore) {
  for (const k of store.keys()) if (k.startsWith(PREFIX)) store.remove(k);
}

/** One account's copy of its lists and its waiting changes. */
export function deviceCopy(user: string, store: Store = browserStore) {
  const listKey = (week: string) => `${PREFIX}${user}:list:${week}`;
  const pendingKey = `${PREFIX}${user}:pending`;
  // A device that remembered this account before sign-ins had ids gets one now, so it keeps working offline;
  // after signing out nothing is remembered, so none is made.
  if (!store.get(SIGN_IN) && rememberedMe(store)?.id === user) store.set(SIGN_IN, newSignIn());
  const signIn = store.get(SIGN_IN);
  // Only within the sign-in this copy was made in: an answer still on its way when someone signs out must not put
  // the household's list back on the device, nor change a later sign-in's copy, even for the same account.
  const mine = () => !!signIn && store.get(SIGN_IN) === signIn && rememberedMe(store)?.id === user;
  return {
    save(list: ShoppingList, now = Date.now()) {
      if (mine())
        store.set(listKey(list.week), JSON.stringify({ list, savedAt: now } satisfies Saved));
    },
    saved: (week: string) => read<Saved>(store, listKey(week)),
    pending: () => read<Change[]>(store, pendingKey) ?? [],
    setPending(changes: Change[]) {
      if (!mine()) return;
      if (changes.length) store.set(pendingKey, JSON.stringify(changes));
      else store.remove(pendingKey);
    },
  };
}

/** `changes` with `next` added: a later tick of the same thing replaces the earlier one, and ticking something
 * added here (not yet sent) ticks it as it is sent. */
export function queue(changes: Change[], next: Change): Change[] {
  if (next.kind === "tick_extra") {
    const added = changes.find((c) => c.kind === "add_extra" && c.id === next.id);
    if (added) return changes.map((c) => (c === added ? { ...added, checked: next.checked } : c));
  }
  const same = (c: Change) =>
    (next.kind === "tick" && c.kind === "tick" && c.week === next.week && c.key === next.key) ||
    (next.kind === "tick_extra" && c.kind === "tick_extra" && c.id === next.id);
  return [...changes.filter((c) => !same(c)), next];
}

/** The list as it looks with this device's waiting changes on it. */
export function withChanges(list: ShoppingList, changes: Change[]): ShoppingList {
  const mine = changes.filter((c) => c.week === list.week);
  if (!mine.length) return list;
  const ticks = new Map(
    mine.flatMap((c) => (c.kind === "tick" ? [[c.key, c.checked] as const] : [])),
  );
  const extraTicks = new Map(
    mine.flatMap((c) => (c.kind === "tick_extra" ? [[c.id, c.checked] as const] : [])),
  );
  const added = mine.flatMap((c) =>
    c.kind === "add_extra" ? [{ id: c.id, name: c.name, checked: c.checked }] : [],
  );
  return {
    ...list,
    sections: list.sections.map((s) => ({
      ...s,
      items: s.items.map((i) => (ticks.has(i.key) ? { ...i, checked: ticks.get(i.key)! } : i)),
    })),
    extras: [
      ...list.extras.map((x) =>
        extraTicks.has(x.id) ? { ...x, checked: extraTicks.get(x.id)! } : x,
      ),
      ...added,
    ],
  };
}

export const isLocal = (id: string) => id.startsWith("local-");

/** A fresh id for something added on this device, until the server gives it one. */
export const localId = () => `local-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

/** What sending one change came to: the list it answered with (and a change still to make, when only part of it
 * went through), refused for good (it will never apply), or the server out of reach (try again later). */
export type Sent = { list: ShoppingList; then?: Change } | "refused" | "unreachable";

/** Send waiting changes in order, stopping at the first that can't reach the server. Returns what is still
 * waiting, the last list the server answered with, and how many were refused. */
export async function sendChanges(changes: Change[], send: (c: Change) => Promise<Sent>) {
  const todo = [...changes];
  let list: ShoppingList | undefined;
  let refused = 0;
  while (todo.length) {
    const r = await send(todo[0]);
    if (r === "unreachable") return { remaining: todo, list, refused };
    todo.shift();
    if (r === "refused") refused++;
    else {
      list = r.list;
      if (r.then) todo.unshift(r.then);
    }
  }
  return { remaining: todo, list, refused };
}
